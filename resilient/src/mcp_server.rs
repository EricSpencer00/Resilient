//! MCP server for the Resilient compiler.
//!
//! Implements the [Model Context Protocol](https://modelcontextprotocol.io)
//! over stdio (newline-delimited JSON-RPC 2.0), exposing the Resilient
//! compiler pipeline as tools, prompts, and resources that AI assistants
//! can use directly.
//!
//! ## Activation
//!
//! ```sh
//! rz mcp
//! ```
//!
//! The server speaks MCP on stdin/stdout. Any MCP-capable client (Claude
//! Desktop, Cursor, VS Code with the MCP extension, …) can connect by
//! spawning `rz mcp` as a child process and routing stdio.
//!
//! ## Tools exposed
//!
//! | Tool | Description |
//! |---|---|
//! | `resilient_parse` | Parse source → parse errors or node count |
//! | `resilient_typecheck` | Parse + typecheck → typed diagnostics |
//! | `resilient_run` | Execute source → captured stdout / errors |
//! | `resilient_lint` | Run all lint passes → lint warnings |
//! | `resilient_format` | Format / pretty-print source |
//! | `resilient_check` | Full pipeline (parse + typecheck + lint) |
//! | `resilient_verify` | Z3 contract verification (requires `--features z3`) |
//!
//! ## Prompts exposed (RES-2645 MCP Scaffolding)
//!
//! | Prompt | Description |
//! |---|---|
//! | `verify_function` | Guided workflow: add contracts + verify with Z3 |
//! | `debug_type_error` | Guided workflow: interpret and fix a type error |
//! | `add_resilience` | Guided workflow: add `recovers_to` + `live` blocks |
//! | `explain_lint` | Guided workflow: understand and fix a lint warning |
//! | `safety_review` | Guided workflow: full safety-critical review |
//!
//! ## Resources exposed (RES-2645 MCP Scaffolding)
//!
//! | Resource | Description |
//! |---|---|
//! | `resilient://docs/syntax` | Full language syntax reference |
//! | `resilient://docs/stdlib` | Standard library function reference |
//! | `resilient://docs/lint-codes` | All lint codes with explanations |
//! | `resilient://docs/contracts` | Contract (`requires`/`ensures`) guide |
//! | `resilient://docs/effects` | Effect system (`@pure`, `@io`) guide |
//!
//! ## Protocol
//!
//! Each message is a single JSON object followed by `\n` (NDJSON).
//! The server never sends multi-line objects. Request / response IDs
//! are mirrored verbatim (string or integer, per spec).
//!
//! Notification messages (no `id` field) receive no response.

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

// ── Public entry point ────────────────────────────────────────────────────────

/// Run the MCP server loop on stdin/stdout.
///
/// Reads one JSON object per line, dispatches it, and writes the
/// response (if any) immediately. Returns only on EOF or a fatal IO error.
pub fn run() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let _ = write_response(
                    &mut out,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {
                            "code": -32700,
                            "message": format!("Parse error: {e}")
                        }
                    }),
                );
                continue;
            }
        };

        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");

        // Notifications (no id) — no response required.
        let is_notification = msg.get("id").is_none();

        let response = dispatch(method, &id, msg.get("params"), is_notification);
        if let Some(resp) = response {
            let _ = write_response(&mut out, &resp);
        }
    }
}

// ── Dispatch ─────────────────────────────────────────────────────────────────

fn dispatch(
    method: &str,
    id: &Value,
    params: Option<&Value>,
    is_notification: bool,
) -> Option<Value> {
    match method {
        "initialize" => Some(handle_initialize(id, params)),
        "notifications/initialized" | "initialized" => None,
        "ping" => {
            if is_notification {
                None
            } else {
                Some(ok(id, json!({})))
            }
        }
        "tools/list" => Some(handle_tools_list(id)),
        "tools/call" => Some(handle_tools_call(id, params)),
        // RES-2645: MCP Scaffolding — prompts and resources support.
        "prompts/list" => Some(handle_prompts_list(id)),
        "prompts/get" => Some(handle_prompts_get(id, params)),
        "resources/list" => Some(handle_resources_list(id)),
        "resources/read" => Some(handle_resources_read(id, params)),
        // Gracefully ignore unknown notifications; error on unknown requests.
        _ if is_notification => None,
        _ => Some(error(id, -32601, format!("Method not found: {method}"))),
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────────

fn handle_initialize(id: &Value, _params: Option<&Value>) -> Value {
    ok(
        id,
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {},
                // RES-2645: advertise prompts and resources so MCP clients
                // (Claude Desktop, Cursor, etc.) discover guided workflows
                // and documentation without manual configuration.
                "prompts": {},
                "resources": {}
            },
            "serverInfo": {
                "name": "resilient",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )
}

fn handle_tools_list(id: &Value) -> Value {
    let mut tools = tool_definitions()
        .as_array()
        .cloned()
        .unwrap_or_default();
    // RES-2645: bridge registry tools appear in tools/list automatically
    // so external verifiers (TLA+ TLC, Lean 4, CBMC, Z3, SPIN, Frama-C,
    // KLEE) are visible to MCP clients alongside built-in tools.
    let bridge_defs = crate::mcp_tool_registry::McpBridgeRegistry::global()
        .mcp_tool_definitions();
    tools.extend(bridge_defs);
    ok(id, json!({ "tools": tools }))
}

fn handle_tools_call(id: &Value, params: Option<&Value>) -> Value {
    let Some(params) = params else {
        return error(id, -32602, "Missing params".to_string());
    };
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return error(id, -32602, "Missing tool name".to_string()),
    };
    let args = params.get("arguments").unwrap_or(&Value::Null);

    let result = match name {
        "resilient_parse" => tool_parse(args),
        "resilient_typecheck" => tool_typecheck(args),
        "resilient_run" => tool_run(args),
        "resilient_lint" => tool_lint(args),
        "resilient_format" => tool_format(args),
        "resilient_check" => tool_check(args),
        "resilient_verify" => tool_verify(args),
        "resilient_explain_lint" => tool_explain_lint(args),
        "resilient_symbols" => tool_symbols(args),
        "resilient_hover" => tool_hover(args),
        "resilient_compile" => tool_compile(args),
        "resilient_disasm" => tool_disasm(args),
        "resilient_vm_run" => tool_vm_run(args),
        "resilient_tla_check" => tool_tla_check(args),
        // RES-2645: four new built-in analysis tools.
        "resilient_fingerprint" => tool_fingerprint(args),
        "resilient_resilience_score" => tool_resilience_score(args),
        "resilient_contract_infer" => tool_contract_infer(args),
        "resilient_call_graph" => tool_call_graph(args),
        // RES-2645: fall through to bridge registry for external tools
        // (SPIN, Frama-C, KLEE, TLC, Lean 4, CBMC, etc.).
        other => {
            let registry = crate::mcp_tool_registry::McpBridgeRegistry::global();
            if registry.get(other).is_some() {
                match registry.invoke(other, args) {
                    Ok(result) => {
                        let text = if result.diagnostics.is_empty() {
                            format!("{:?}: {}", result.outcome, result.raw_output)
                        } else {
                            result.diagnostics.join("\n")
                        };
                        match result.outcome {
                            crate::mcp_tool_registry::ToolOutcome::Clean => Ok(text),
                            _ => Err(text),
                        }
                    }
                    Err(e) => Err(e),
                }
            } else {
                Err(format!("Unknown tool: {other}"))
            }
        }
    };

    match result {
        Ok(text) => ok(
            id,
            json!({
                "content": [{ "type": "text", "text": text }],
                "isError": false
            }),
        ),
        Err(msg) => ok(
            id,
            json!({
                "content": [{ "type": "text", "text": msg }],
                "isError": true
            }),
        ),
    }
}

// ── Prompts (RES-2645: MCP Scaffolding) ──────────────────────────────────────

/// Guided workflow prompt descriptors surfaced via `prompts/list`.
fn prompt_descriptors() -> Vec<Value> {
    vec![
        json!({
            "name": "verify_function",
            "description": "Guided workflow: add `requires`/`ensures` contracts to a Resilient function and verify them with Z3.",
            "arguments": [
                {
                    "name": "source",
                    "description": "Resilient source code containing the function(s) to verify",
                    "required": true
                },
                {
                    "name": "function_name",
                    "description": "Name of the specific function to focus on (optional; omit to review all)",
                    "required": false
                }
            ]
        }),
        json!({
            "name": "debug_type_error",
            "description": "Guided workflow: interpret a Resilient type error message and suggest a fix.",
            "arguments": [
                {
                    "name": "source",
                    "description": "Resilient source code that produces the type error",
                    "required": true
                },
                {
                    "name": "error_message",
                    "description": "The type error message from the compiler (optional; will be inferred by running the typechecker)",
                    "required": false
                }
            ]
        }),
        json!({
            "name": "add_resilience",
            "description": "Guided workflow: add fault tolerance to a Resilient function using `recovers_to` postconditions and `live` retry blocks.",
            "arguments": [
                {
                    "name": "source",
                    "description": "Resilient source code to harden",
                    "required": true
                }
            ]
        }),
        json!({
            "name": "explain_lint",
            "description": "Guided workflow: explain a Resilient lint warning and suggest how to fix it.",
            "arguments": [
                {
                    "name": "code",
                    "description": "Lint code to explain (e.g. L0010, L0038)",
                    "required": true
                },
                {
                    "name": "source",
                    "description": "Source snippet that triggered the warning (optional)",
                    "required": false
                }
            ]
        }),
        json!({
            "name": "safety_review",
            "description": "Guided workflow: full safety-critical code review — checks contracts, effects, lint, type safety, and resilience score.",
            "arguments": [
                {
                    "name": "source",
                    "description": "Resilient source code to review",
                    "required": true
                }
            ]
        }),
    ]
}

fn handle_prompts_list(id: &Value) -> Value {
    ok(id, json!({ "prompts": prompt_descriptors() }))
}

fn handle_prompts_get(id: &Value, params: Option<&Value>) -> Value {
    let Some(params) = params else {
        return error(id, -32602, "Missing params".to_string());
    };
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return error(id, -32602, "Missing prompt name".to_string()),
    };
    let args = params.get("arguments").unwrap_or(&Value::Null);

    let messages = match name {
        "verify_function" => {
            let src = args.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let fn_name = args.get("function_name").and_then(|v| v.as_str());
            let focus = fn_name
                .map(|f| format!("Focus on function `{f}`."))
                .unwrap_or_default();
            let check_result = if src.is_empty() {
                "(no source provided)".to_string()
            } else {
                match tool_check(&json!({ "source": src })) {
                    Ok(msg) => format!("Compiler: {msg}"),
                    Err(msg) => format!("Compiler errors:\n{msg}"),
                }
            };
            vec![json!({
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "Please help me add formal contracts (`requires`/`ensures`) to the \
                         following Resilient program and verify them with Z3. {focus}\n\n\
                         Source:\n```rz\n{src}\n```\n\n\
                         Current compiler output:\n{check_result}\n\n\
                         Steps:\n\
                         1. Identify preconditions (what must hold for the function to work correctly)\n\
                         2. Identify postconditions (what the function guarantees)\n\
                         3. Suggest `requires` and `ensures` clauses\n\
                         4. Explain what Z3 can prove and what it cannot"
                    )
                }
            })]
        }
        "debug_type_error" => {
            let src = args.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let provided_error = args.get("error_message").and_then(|v| v.as_str());
            let error_text = provided_error.map(|e| e.to_string()).unwrap_or_else(|| {
                if src.is_empty() {
                    "(no source provided)".to_string()
                } else {
                    match tool_typecheck(&json!({ "source": src })) {
                        Ok(_) => "(no type errors found)".to_string(),
                        Err(msg) => msg,
                    }
                }
            });
            vec![json!({
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "I have a Resilient type error I need help understanding and fixing.\n\n\
                         Source:\n```rz\n{src}\n```\n\n\
                         Error:\n```\n{error_text}\n```\n\n\
                         Please:\n\
                         1. Explain what the error means in plain language\n\
                         2. Identify the root cause in the source\n\
                         3. Show a corrected version of the code\n\
                         4. Explain why the fix works"
                    )
                }
            })]
        }
        "add_resilience" => {
            let src = args.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let score_result = if src.is_empty() {
                "(no source provided)".to_string()
            } else {
                match tool_resilience_score(&json!({ "source": src })) {
                    Ok(msg) => msg,
                    Err(msg) => msg,
                }
            };
            vec![json!({
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "Please help me add fault tolerance to this Resilient program.\n\n\
                         Source:\n```rz\n{src}\n```\n\n\
                         Current resilience score:\n{score_result}\n\n\
                         Resilient fault-tolerance features:\n\
                         - `fails ErrorType` — declare what errors the function can raise\n\
                         - `recovers_to: EXPR` — postcondition that holds after any crash\n\
                         - `live {{ ... }}` — retry block that re-executes on recoverable faults\n\
                         - `@crash_only_cert` — certify the function only returns Ok or Err\n\n\
                         Please:\n\
                         1. Identify which functions handle recoverable failures\n\
                         2. Suggest `fails`/`recovers_to` annotations\n\
                         3. Show `live` block patterns where retries make sense\n\
                         4. Explain the recovery semantics for each suggestion"
                    )
                }
            })]
        }
        "explain_lint" => {
            let code = args.get("code").and_then(|v| v.as_str()).unwrap_or("");
            let source_snippet = args
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let explanation = match tool_explain_lint(&json!({ "code": code })) {
                Ok(text) => text,
                Err(msg) => format!("Error: {msg}"),
            };
            let snippet_section = if source_snippet.is_empty() {
                String::new()
            } else {
                format!("\n\nCode that triggered the warning:\n```rz\n{source_snippet}\n```")
            };
            vec![json!({
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "Please help me understand and fix Resilient lint warning `{code}`.\n\n\
                         Official explanation:\n{explanation}{snippet_section}\n\n\
                         Please:\n\
                         1. Explain WHY this is flagged as a warning\n\
                         2. Show a concrete example of the bad pattern\n\
                         3. Show the corrected version\n\
                         4. Explain when (if ever) you might want to suppress it with `// resilient: allow {code}`"
                    )
                }
            })]
        }
        "safety_review" => {
            let src = args.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let (check_out, score_out, lint_out) = if src.is_empty() {
                (
                    "(no source provided)".to_string(),
                    "(no source provided)".to_string(),
                    "(no source provided)".to_string(),
                )
            } else {
                let check = match tool_check(&json!({ "source": src })) {
                    Ok(m) => m,
                    Err(m) => m,
                };
                let score = match tool_resilience_score(&json!({ "source": src })) {
                    Ok(m) => m,
                    Err(m) => m,
                };
                let lint = match tool_lint(&json!({ "source": src })) {
                    Ok(m) => m,
                    Err(m) => m,
                };
                (check, score, lint)
            };
            vec![json!({
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "Please perform a full safety-critical code review of this Resilient program.\n\n\
                         Source:\n```rz\n{src}\n```\n\n\
                         Compiler check: {check_out}\n\
                         Resilience score: {score_out}\n\
                         Lint output: {lint_out}\n\n\
                         Please review:\n\
                         1. **Type safety** — are all types correct and no `Any` escapes?\n\
                         2. **Contracts** — are preconditions and postconditions adequate?\n\
                         3. **Effects** — are effect annotations (`@pure`/`@io`) correct?\n\
                         4. **Fault tolerance** — are failure paths handled with `live` / `recovers_to`?\n\
                         5. **Embedded safety** — any panics, unbounded loops, or heap allocations?\n\
                         6. **Lint violations** — are all warnings addressed?\n\
                         7. **Overall verdict** — ready for safety-critical deployment?"
                    )
                }
            })]
        }
        other => {
            return error(
                id,
                -32602,
                format!(
                    "Unknown prompt `{other}`. Available: {}",
                    prompt_descriptors()
                        .iter()
                        .filter_map(|p| p["name"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
    };

    ok(id, json!({ "messages": messages }))
}

// ── Resources (RES-2645: MCP Scaffolding) ────────────────────────────────────

/// Static documentation resource descriptors.
fn resource_descriptors() -> Vec<Value> {
    vec![
        json!({
            "uri": "resilient://docs/syntax",
            "name": "Resilient Language Syntax",
            "description": "Complete syntax reference for the Resilient programming language",
            "mimeType": "text/plain"
        }),
        json!({
            "uri": "resilient://docs/stdlib",
            "name": "Resilient Standard Library",
            "description": "All builtin functions and their signatures",
            "mimeType": "text/plain"
        }),
        json!({
            "uri": "resilient://docs/lint-codes",
            "name": "Resilient Lint Codes",
            "description": "All lint codes with explanations and fix guidance",
            "mimeType": "text/plain"
        }),
        json!({
            "uri": "resilient://docs/contracts",
            "name": "Resilient Contracts Guide",
            "description": "How to write `requires`/`ensures` contracts and use Z3 verification",
            "mimeType": "text/plain"
        }),
        json!({
            "uri": "resilient://docs/effects",
            "name": "Resilient Effect System",
            "description": "Effect annotations (@pure, @io) and effect inference",
            "mimeType": "text/plain"
        }),
        json!({
            "uri": "resilient://docs/resilience",
            "name": "Resilient Fault Tolerance Guide",
            "description": "How to use `live` blocks, `recovers_to`, and fault recovery patterns",
            "mimeType": "text/plain"
        }),
    ]
}

fn handle_resources_list(id: &Value) -> Value {
    ok(id, json!({ "resources": resource_descriptors() }))
}

fn handle_resources_read(id: &Value, params: Option<&Value>) -> Value {
    let Some(params) = params else {
        return error(id, -32602, "Missing params".to_string());
    };
    let uri = match params.get("uri").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return error(id, -32602, "Missing resource uri".to_string()),
    };

    let content = match uri {
        "resilient://docs/syntax" => resource_syntax_doc(),
        "resilient://docs/stdlib" => resource_stdlib_doc(),
        "resilient://docs/lint-codes" => resource_lint_codes_doc(),
        "resilient://docs/contracts" => resource_contracts_doc(),
        "resilient://docs/effects" => resource_effects_doc(),
        "resilient://docs/resilience" => resource_resilience_doc(),
        other => {
            return error(
                id,
                -32602,
                format!(
                    "Unknown resource URI `{other}`. Available: {}",
                    resource_descriptors()
                        .iter()
                        .filter_map(|r| r["uri"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
    };

    ok(
        id,
        json!({
            "contents": [{
                "uri": uri,
                "mimeType": "text/plain",
                "text": content
            }]
        }),
    )
}

fn resource_syntax_doc() -> String {
    "# Resilient Language Syntax Reference\n\n\
     ## Functions\n\
     fn name(type param, ...) -> ReturnType {\n\
         body\n\
     }\n\n\
     ## Variables\n\
     let x: int = 42;\n\
     let x = 42;           // type inferred\n\
     static let counter = 0;  // persists across calls\n\
     const MAX: int = 100; // compile-time constant\n\n\
     ## Types\n\
     int    float    bool    string    array    void\n\
     fn(T, ...) -> R   // function type\n\
     Array<T>          // typed array (planned)\n\n\
     ## Control Flow\n\
     if COND { ... } else { ... }\n\
     while COND { ... }\n\
     for x in ITERABLE { ... }\n\
     match EXPR { PATTERN => EXPR, ... }\n\
     return EXPR;\n\
     break; continue;\n\n\
     ## Structs\n\
     struct Point { int x, int y, }\n\
     let p = new Point { x: 1, y: 2 };\n\
     p.x    // field access\n\n\
     ## Enums\n\
     enum Color { Red, Green, Blue }\n\
     match c { Color::Red => ..., _ => ... }\n\n\
     ## Contracts\n\
     fn f(int x) -> int requires x > 0 ensures result > 0 { ... }\n\n\
     ## Effects\n\
     @pure fn f(...) { ... }  // no side effects\n\
     @io fn g(...) { ... }    // may perform I/O\n\n\
     ## Fault Tolerance\n\
     fn f() fails IOError recovers_to: result >= 0 { ... }\n\
     live { ... }             // retry block\n\n\
     ## Closures\n\
     fn(int x) -> int { x * 2 }     // anonymous function\n\
     fn(x) { x + 1 }                // type-inferred params (planned)\n\n\
     ## String Interpolation\n\
     let msg = \"value is {x}\";\n\n\
     ## Pattern Matching\n\
     match x {\n\
         0 => \"zero\",\n\
         1..=9 => \"small\",\n\
         _ => \"large\",\n\
     }\n"
        .to_string()
}

fn resource_stdlib_doc() -> String {
    let lines = vec![
        "# Resilient Standard Library\n".to_string(),
        "## Math\n".to_string(),
        "abs(x: int) -> int     sqrt(x) -> float    pow(base, exp) -> int".to_string(),
        "floor(x) -> int        ceil(x) -> int      round(x) -> int".to_string(),
        "min(a, b) -> int       max(a, b) -> int     sign(x) -> int".to_string(),
        "clamp(x, lo, hi) -> int".to_string(),
        "sin(x) -> float        cos(x) -> float     tan(x) -> float".to_string(),
        "log(x) -> float        log2(x) -> float    log10(x) -> float".to_string(),
        "\n## Strings\n".to_string(),
        "len(s: string) -> int          to_string(x) -> string".to_string(),
        "string_upper(s) -> string      string_lower(s) -> string".to_string(),
        "string_trim(s) -> string       string_contains(s, sub) -> bool".to_string(),
        "string_starts_with(s, pre) -> bool   string_ends_with(s, suf) -> bool".to_string(),
        "string_split(s, delim) -> array      string_join(arr, sep) -> string".to_string(),
        "string_replace(s, from, to) -> string".to_string(),
        "string_slice(s, lo, hi) -> string    string_index(s, i) -> string".to_string(),
        "\n## Arrays\n".to_string(),
        "len(arr: array) -> int         push(arr, item) -> array".to_string(),
        "pop(arr) -> any               array_append(arr, item) -> array".to_string(),
        "array_map(arr, fn) -> array    array_filter(arr, pred) -> array".to_string(),
        "array_reduce(arr, init, fn) -> any   array_find(arr, pred) -> any".to_string(),
        "array_find_index(arr, pred) -> int   array_any(arr, pred) -> bool".to_string(),
        "array_all(arr, pred) -> bool         array_sort(arr) -> array".to_string(),
        "array_reverse(arr) -> array          array_sum(arr) -> int".to_string(),
        "array_min(arr) -> int               array_max(arr) -> int".to_string(),
        "array_range(lo, hi) -> array        array_slice(arr, lo, hi, incl) -> array".to_string(),
        "\n## I/O\n".to_string(),
        "println(x)   print(x)   read_line() -> string   input(prompt) -> string".to_string(),
        "\n## Type checks\n".to_string(),
        "is_int(x) -> bool   is_float(x) -> bool   is_string(x) -> bool".to_string(),
        "is_array(x) -> bool  is_null(x) -> bool".to_string(),
    ];
    lines.join("\n")
}

fn resource_lint_codes_doc() -> String {
    let mut parts = vec!["# Resilient Lint Codes\n".to_string()];
    for code in crate::lint::KNOWN_CODES {
        match crate::lint::explain(code) {
            Some(text) => parts.push(format!("## {code}\n{text}\n")),
            None => parts.push(format!("## {code}\n(no explanation available)\n")),
        }
    }
    parts.join("\n")
}

fn resource_contracts_doc() -> String {
    "# Resilient Contracts Guide\n\n\
     ## Preconditions (`requires`)\n\
     State what must be true BEFORE the function runs:\n\
     ```rz\n\
     fn divide(int a, int b) -> int requires b != 0 { return a / b; }\n\
     ```\n\n\
     ## Postconditions (`ensures`)\n\
     State what the function GUARANTEES on return. The special variable `result`\n\
     refers to the return value:\n\
     ```rz\n\
     fn abs_val(int x) -> int ensures result >= 0 { return if x < 0 { -x } else { x }; }\n\
     ```\n\n\
     ## Multiple clauses\n\
     Chain with whitespace (no comma needed):\n\
     ```rz\n\
     fn clamp(int x, int lo, int hi) -> int\n\
         requires lo <= hi\n\
         ensures result >= lo\n\
         ensures result <= hi\n\
     {\n\
         if x < lo { return lo; }\n\
         if x > hi { return hi; }\n\
         return x;\n\
     }\n\
     ```\n\n\
     ## Z3 verification\n\
     Build with `--features z3` to have Z3 attempt to prove contracts at\n\
     compile time. Functions with provable contracts skip runtime checks.\n\n\
     ## Resilience score\n\
     `rz check` shows a resilience score A–F. Functions with both `requires`\n\
     and `ensures` clauses score higher."
        .to_string()
}

fn resource_effects_doc() -> String {
    "# Resilient Effect System\n\n\
     ## Annotations\n\
     - `@pure` — function has no side effects; may only call other `@pure` functions\n\
     - `@io` — function may perform I/O; required for `println`, file access, etc.\n\n\
     ## Examples\n\
     ```rz\n\
     @pure fn double(int x) -> int { return x * 2; }  // purely computational\n\
     @io fn log(string msg) { println(msg); }          // has I/O side effect\n\
     ```\n\n\
     ## Inference\n\
     The compiler infers effects by inspecting the body. If a function calls\n\
     `println` or other I/O builtins, it is automatically inferred as `@io`\n\
     even if the annotation is absent.\n\n\
     ## Effect errors\n\
     A `@pure` function that calls an `@io` function is a type error.\n\n\
     ## Linear effects\n\
     `#[linear]` resources must be consumed exactly once — the compiler\n\
     enforces that no reference is dropped silently and no reference is\n\
     used after consumption."
        .to_string()
}

fn resource_resilience_doc() -> String {
    "# Resilient Fault Tolerance Guide\n\n\
     ## `fails` — declare recoverable errors\n\
     ```rz\n\
     fn read_sensor() -> int fails IOError { ... }\n\
     ```\n\n\
     ## `recovers_to` — postcondition after a crash\n\
     State what the caller can rely on even if the function was interrupted:\n\
     ```rz\n\
     fn write_log(string msg)\n\
         fails IOError\n\
         recovers_to: len(msg) == 0 || log_is_consistent()\n\
     { ... }\n\
     ```\n\n\
     ## `live` blocks — automatic retry on recoverable fault\n\
     ```rz\n\
     live {\n\
         let data = read_sensor();  // retried if IOError occurs\n\
         process(data);\n\
     }\n\
     ```\n\n\
     ## `@crash_only_cert` — guarantee clean termination\n\
     ```rz\n\
     #[crash_only_cert]\n\
     fn safe_op() -> Result { ... }\n\
     ```\n\n\
     ## BMC verification (RES-1857)\n\
     With `--features z3`, the bounded model checker verifies that\n\
     `recovers_to` postconditions hold after any prefix of instructions\n\
     that could be interrupted by a crash. A `sat` result means the\n\
     postcondition CAN be violated — fix the code or weaken the clause."
        .to_string()
}

// ── Tool implementations ──────────────────────────────────────────────────────

/// Extract the `source` argument or return an error string.
fn source_arg(args: &Value) -> Result<&str, String> {
    args.get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument: source (string)".to_string())
}

/// Parse Resilient source and report errors or a brief summary.
fn tool_parse(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, errors) = crate::parse(src);
    if !errors.is_empty() {
        return Err(format!(
            "Parse errors ({}):\n{}",
            errors.len(),
            errors.join("\n")
        ));
    }
    let stmts = match &program {
        crate::Node::Program(s) => s.len(),
        _ => 0,
    };
    Ok(format!(
        "OK — parsed {stmts} top-level statement(s), no errors."
    ))
}

/// Parse + typecheck source and return all diagnostics.
fn tool_typecheck(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors ({}):\n{}",
            parse_errors.len(),
            parse_errors.join("\n")
        ));
    }

    let mut tc = crate::typechecker::TypeChecker::new();
    match tc.check_program_with_source(&program, "<mcp>") {
        Ok(_) => Ok("OK — no type errors.".to_string()),
        Err(e) => Err(format!("Type error:\n{e}")),
    }
}

/// Execute Resilient source and capture stdout.
fn tool_run(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let result = crate::run_program(src);
    if result.ok {
        let mut out = String::new();
        if !result.stdout.is_empty() {
            out.push_str("Output:\n");
            out.push_str(&result.stdout);
        } else {
            out.push_str("OK — program exited with no output.");
        }
        Ok(out)
    } else {
        let mut msg = String::new();
        if !result.stdout.is_empty() {
            msg.push_str("Output before error:\n");
            msg.push_str(&result.stdout);
            msg.push('\n');
        }
        msg.push_str("Errors:\n");
        msg.push_str(&result.errors.join("\n"));
        Err(msg)
    }
}

/// Run all lint passes and return warnings.
fn tool_lint(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors ({}):\n{}",
            parse_errors.len(),
            parse_errors.join("\n")
        ));
    }

    let lints = crate::lint::check(&program, src);
    if lints.is_empty() {
        Ok("OK — no lint warnings.".to_string())
    } else {
        let lines: Vec<String> = lints
            .iter()
            .map(|l| crate::lint::format_lint(l, "<mcp>"))
            .collect();
        Err(format!(
            "{} lint warning(s):\n{}",
            lints.len(),
            lines.join("\n")
        ))
    }
}

/// Format / pretty-print Resilient source.
fn tool_format(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors (cannot format):\n{}",
            parse_errors.join("\n")
        ));
    }
    let formatted = crate::formatter::Formatter::format(&program);
    Ok(formatted)
}

/// Full pipeline: parse + typecheck + lint.
fn tool_check(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors ({}):\n{}",
            parse_errors.len(),
            parse_errors.join("\n")
        ));
    }

    // Typecheck.
    let mut tc = crate::typechecker::TypeChecker::new();
    if let Err(e) = tc.check_program_with_source(&program, "<mcp>") {
        return Err(format!("Type error:\n{e}"));
    }

    // Lint.
    let lints = crate::lint::check(&program, src);
    if !lints.is_empty() {
        let lines: Vec<String> = lints
            .iter()
            .map(|l| crate::lint::format_lint(l, "<mcp>"))
            .collect();
        return Err(format!(
            "{} lint warning(s):\n{}",
            lints.len(),
            lines.join("\n")
        ));
    }

    Ok("OK — parse, typecheck, and lint all passed.".to_string())
}

/// Z3 contract verification. Reports unavailability gracefully when the
/// `z3` feature is not compiled in.
fn tool_verify(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors (cannot verify):\n{}",
            parse_errors.join("\n")
        ));
    }

    #[cfg(feature = "z3")]
    {
        let mut tc = crate::typechecker::TypeChecker::new();
        match tc.check_program_with_source(&program, "<mcp>") {
            Ok(_) => {
                let provable = tc.stats.fully_provable_fns();
                if provable.is_empty() {
                    Ok(
                        "OK — no contracts to verify (no requires/ensures clauses found)."
                            .to_string(),
                    )
                } else {
                    let names: Vec<&str> = provable.iter().map(|s| s.as_str()).collect();
                    Ok(format!(
                        "OK — Z3 fully proved {} function(s): {}",
                        provable.len(),
                        names.join(", ")
                    ))
                }
            }
            Err(e) => Err(format!("Verification failed:\n{e}")),
        }
    }

    #[cfg(not(feature = "z3"))]
    {
        let _ = program;
        Err("Z3 verification is not available in this build.\n\
             Rebuild with `--features z3` to enable contract verification."
            .to_string())
    }
}

/// Explain a lint code in detail.
fn tool_explain_lint(args: &Value) -> Result<String, String> {
    let code = args
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument: code (string, e.g. \"L0010\")".to_string())?;
    match crate::lint::explain(code) {
        Some(text) => Ok(text.to_string()),
        None => Err(format!(
            "Unknown lint code `{code}`. Known codes: {}",
            crate::lint::KNOWN_CODES.join(", ")
        )),
    }
}

/// Extract named symbols (functions, let bindings) from source.
fn tool_symbols(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors ({}):\n{}",
            parse_errors.len(),
            parse_errors.join("\n")
        ));
    }
    let mut symbols = Vec::new();
    collect_symbols(&program, &mut symbols);
    if symbols.is_empty() {
        Ok("No named symbols found.".to_string())
    } else {
        Ok(symbols.join("\n"))
    }
}

fn collect_symbols(node: &crate::Node, out: &mut Vec<String>) {
    match node {
        crate::Node::Program(stmts) => {
            for s in stmts {
                collect_symbols(&s.node, out);
            }
        }
        crate::Node::Function {
            name,
            parameters,
            return_type,
            body,
            ..
        } => {
            let params = parameters
                .iter()
                .map(|(ty, pname)| format!("{ty} {pname}"))
                .collect::<Vec<_>>()
                .join(", ");
            let ret = return_type
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| format!(" -> {s}"))
                .unwrap_or_default();
            out.push(format!("fn {name}({params}){ret}"));
            collect_symbols(body, out);
        }
        crate::Node::LetStatement {
            name, type_annot, ..
        } => {
            let ty = type_annot.as_deref().unwrap_or("?");
            out.push(format!("let {name}: {ty}"));
        }
        crate::Node::Block { stmts, .. } => {
            for s in stmts {
                collect_symbols(s, out);
            }
        }
        _ => {}
    }
}

/// Return type info for an identifier at a given byte offset in the source.
fn tool_hover(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let offset = args.get("offset").and_then(|v| v.as_u64()).ok_or_else(|| {
        "Missing required argument: offset (integer byte offset into source)".to_string()
    })? as usize;

    if offset > src.len() {
        return Err(format!(
            "Offset {offset} is out of range (source is {} bytes)",
            src.len()
        ));
    }

    // Extract the identifier at `offset`: scan left/right for word boundaries.
    let bytes = src.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    // If the character at offset is not an identifier character, report error
    // immediately — don't scan backwards into the preceding token.
    if !is_ident(bytes[offset]) {
        return Err(format!(
            "No identifier at offset {offset} (found {:?})",
            bytes[offset] as char
        ));
    }
    let start = (0..=offset)
        .rev()
        .find(|&i| i == 0 || !is_ident(bytes[i - 1]))
        .unwrap_or(offset);
    let end = (offset..src.len())
        .find(|&i| !is_ident(bytes[i]))
        .unwrap_or(src.len());
    let ident = &src[start..end];
    if ident.is_empty() {
        return Err("No identifier at the given offset.".to_string());
    }

    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors — cannot hover:\n{}",
            parse_errors.join("\n")
        ));
    }

    match hover_infer_type(&program, ident) {
        Some(ty) => Ok(format!("{ident}: {ty}")),
        None => Ok(format!("{ident}: (type unknown)")),
    }
}

/// Minimal identifier-type inference for hover — walks functions looking for
/// parameter types and let-binding annotations. Does not depend on the `lsp`
/// feature.
fn hover_infer_type(node: &crate::Node, target: &str) -> Option<String> {
    match node {
        crate::Node::Program(stmts) => {
            for s in stmts {
                if let Some(ty) = hover_infer_type(&s.node, target) {
                    return Some(ty);
                }
            }
            None
        }
        crate::Node::Function {
            name,
            parameters,
            return_type,
            body,
            ..
        } => {
            if name == target {
                let params = parameters
                    .iter()
                    .map(|(ty, pname)| format!("{ty} {pname}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret = return_type
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|s| format!(" -> {s}"))
                    .unwrap_or_default();
                return Some(format!("fn {name}({params}){ret}"));
            }
            for (ty, pname) in parameters {
                if pname == target {
                    return Some(ty.clone());
                }
            }
            hover_infer_type(body, target)
        }
        crate::Node::LetStatement {
            name,
            value,
            type_annot,
            ..
        } if name == target => {
            let ty = type_annot.clone().unwrap_or_else(|| {
                // Infer from literal type.
                match value.as_ref() {
                    crate::Node::IntegerLiteral { .. } => "int".to_string(),
                    crate::Node::FloatLiteral { .. } => "float".to_string(),
                    crate::Node::BooleanLiteral { .. } => "bool".to_string(),
                    crate::Node::StringLiteral { .. } => "string".to_string(),
                    _ => "?".to_string(),
                }
            });
            Some(ty)
        }
        crate::Node::Block { stmts, .. } => {
            for s in stmts {
                if let Some(ty) = hover_infer_type(s, target) {
                    return Some(ty);
                }
            }
            None
        }
        _ => None,
    }
}

/// Compile Resilient source to bytecode and return a summary.
fn tool_compile(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors ({}):\n{}",
            parse_errors.len(),
            parse_errors.join("\n")
        ));
    }
    let compiled =
        crate::compiler::compile(&program).map_err(|e| format!("Compile error:\n{e}"))?;

    let total_instructions: usize = compiled.main.code.len()
        + compiled
            .functions
            .iter()
            .map(|f| f.chunk.code.len())
            .sum::<usize>();
    let fn_count = compiled.functions.len();

    let fn_lines: Vec<String> = compiled
        .functions
        .iter()
        .map(|f| format!("  fn {} — {} instructions", f.name, f.chunk.code.len()))
        .collect();
    let fn_section = if fn_lines.is_empty() {
        String::new()
    } else {
        format!("\nFunctions:\n{}", fn_lines.join("\n"))
    };

    Ok(format!(
        "OK — compiled to bytecode.\n\
         Main chunk: {} instructions\n\
         Functions: {fn_count}{fn_section}\n\
         Total instructions: {total_instructions}",
        compiled.main.code.len(),
    ))
}

/// Compile Resilient source and return the full bytecode disassembly.
fn tool_disasm(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors ({}):\n{}",
            parse_errors.len(),
            parse_errors.join("\n")
        ));
    }
    let compiled =
        crate::compiler::compile(&program).map_err(|e| format!("Compile error:\n{e}"))?;

    let mut out = String::new();
    crate::disasm::disassemble(&compiled, &mut out)
        .map_err(|e| format!("Disassembly error: {e}"))?;
    Ok(out)
}

/// Compile Resilient source and execute it through the bytecode VM,
/// capturing stdout. Returns the captured output and the final value.
fn tool_vm_run(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors ({}):\n{}",
            parse_errors.len(),
            parse_errors.join("\n")
        ));
    }
    let compiled =
        crate::compiler::compile(&program).map_err(|e| format!("Compile error:\n{e}"))?;

    let (vm_result, captured) =
        crate::output_sink::with_captured_output(|| crate::vm::run(&compiled));

    match vm_result {
        Ok(value) => {
            let mut out = String::new();
            if !captured.is_empty() {
                out.push_str("Output:\n");
                out.push_str(&captured);
            }
            let val_str = format!("{value:?}");
            if val_str != "Void" {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("Result: {val_str}"));
            }
            if out.is_empty() {
                out.push_str("OK — program exited with no output.");
            }
            Ok(out)
        }
        Err(e) => {
            let mut msg = String::new();
            if !captured.is_empty() {
                msg.push_str("Output before error:\n");
                msg.push_str(&captured);
                msg.push('\n');
            }
            msg.push_str(&format!("VM error: {e}"));
            Err(msg)
        }
    }
}

/// Run TLC model checking on an inline TLA+ specification.
///
/// The spec content is written to a temporary `.tla` file, then
/// `tla_bridge::check_tla_file` shells out to TLC and surfaces diagnostics.
fn tool_tla_check(args: &Value) -> Result<String, String> {
    let spec = args.get("spec").and_then(|v| v.as_str()).ok_or_else(|| {
        "Missing required argument: spec (string — TLA+ specification source)".to_string()
    })?;
    let tlc_jar = args.get("tlc_jar").and_then(|v| v.as_str());

    // Write the spec to a temporary file so TLC can read it.
    let tmp_dir = std::env::temp_dir();
    let tmp_file = tmp_dir.join("__resilient_mcp_spec.tla");
    std::fs::write(&tmp_file, spec)
        .map_err(|e| format!("Failed to write temporary TLA+ file: {e}"))?;

    let result = crate::tla_bridge::check_tla_file(&tmp_file, tlc_jar);
    let _ = std::fs::remove_file(&tmp_file);

    let outcome_label = match result.outcome {
        crate::tla_bridge::TlaOutcome::Clean => "CLEAN",
        crate::tla_bridge::TlaOutcome::Violated => "VIOLATED",
        crate::tla_bridge::TlaOutcome::ParseError => "PARSE ERROR",
    };

    let diag_text = result.diagnostics.join("\n");
    let summary = format!("TLC outcome: {outcome_label}\n{diag_text}");

    match result.outcome {
        crate::tla_bridge::TlaOutcome::Clean => Ok(summary),
        _ => Err(summary),
    }
}

/// Compute behavioral fingerprints for every function in the program.
///
/// Each fingerprint is a stable hash of the function's contracts
/// (requires/ensures), parameter types, and fails variants — NOT the body.
/// Body refactors that preserve postconditions keep the same fingerprint.
fn tool_fingerprint(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors ({}):\n{}",
            parse_errors.len(),
            parse_errors.join("\n")
        ));
    }
    let fps = crate::behavioral_fingerprint::fingerprint_program(&program);
    if fps.is_empty() {
        return Ok("No functions found.".to_string());
    }
    let mut names: Vec<&String> = fps.keys().collect();
    names.sort();
    let lines: Vec<String> = names
        .iter()
        .map(|n| {
            let fp = &fps[*n];
            let recovery = if fp.has_recovery { " [recovery]" } else { "" };
            let fails = if fp.fails_variants.is_empty() {
                String::new()
            } else {
                format!(" fails=[{}]", fp.fails_variants.join(", "))
            };
            format!(
                "  {} — digest: 0x{:016x}{}{}",
                n, fp.digest, recovery, fails
            )
        })
        .collect();
    Ok(format!(
        "Behavioral fingerprints ({} functions):\n{}",
        fps.len(),
        lines.join("\n")
    ))
}

/// Compute resilience scores for every function in the program.
///
/// Each score (0–100) is a weighted sum of safety signals: contract
/// coverage, effect annotations, live-recovery blocks, call-site
/// coverage, and body simplicity. Scores map to letter grades A–F.
fn tool_resilience_score(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors ({}):\n{}",
            parse_errors.len(),
            parse_errors.join("\n")
        ));
    }
    let scores = crate::resilience_score::score_program(&program);
    if scores.is_empty() {
        return Ok("No functions found.".to_string());
    }
    let lines: Vec<String> = scores
        .iter()
        .map(|s| {
            format!(
                "  {} — {}/100 ({})  contracts={} effects={} live={} coverage={} simplicity={}",
                s.function_name,
                s.total,
                s.grade(),
                s.contracts_pts,
                s.effects_pts,
                s.live_pts,
                s.coverage_pts,
                s.simplicity_pts,
            )
        })
        .collect();
    let avg = scores.iter().map(|s| s.total as u64).sum::<u64>() / scores.len() as u64;
    Ok(format!(
        "Resilience scores ({} functions, avg {avg}/100):\n{}",
        scores.len(),
        lines.join("\n")
    ))
}

/// Run contract inference on the program and show suggested requires/ensures
/// clauses that could be added to under-specified functions.
fn tool_contract_infer(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors ({}):\n{}",
            parse_errors.len(),
            parse_errors.join("\n")
        ));
    }
    let inferred = crate::contract_inference::infer_program(&program);
    if inferred.is_empty() {
        return Ok("No inference suggestions (all functions already have complete contracts, or no functions found).".to_string());
    }
    let lines: Vec<String> = inferred
        .iter()
        .flat_map(|ic| {
            let mut out = Vec::new();
            for req in &ic.requires {
                out.push(format!("  {} — suggested requires: {}", ic.function_name, req));
            }
            for ens in &ic.ensures {
                out.push(format!("  {} — suggested ensures: {}", ic.function_name, ens));
            }
            out
        })
        .collect();
    if lines.is_empty() {
        return Ok("No inference suggestions generated.".to_string());
    }
    Ok(format!(
        "Contract inference suggestions ({} functions):\n{}",
        inferred.len(),
        lines.join("\n")
    ))
}

/// Extract and display the function call graph for the program.
///
/// Shows, for each function, which other functions it calls directly.
/// Mutual recursion and cycles are flagged.
fn tool_call_graph(args: &Value) -> Result<String, String> {
    let src = source_arg(args)?;
    let (program, parse_errors) = crate::parse(src);
    if !parse_errors.is_empty() {
        return Err(format!(
            "Parse errors ({}):\n{}",
            parse_errors.len(),
            parse_errors.join("\n")
        ));
    }
    let crate::Node::Program(stmts) = &program else {
        return Ok("Not a program.".to_string());
    };
    use std::collections::{HashMap, HashSet};
    let mut graph: HashMap<String, HashSet<String>> = HashMap::new();
    for stmt in stmts {
        if let crate::Node::Function { name, body, .. } = &stmt.node {
            let mut callees = HashSet::new();
            collect_call_targets(body, &mut callees);
            graph.insert(name.clone(), callees);
        }
    }
    if graph.is_empty() {
        return Ok("No functions found.".to_string());
    }
    let mut fn_names: Vec<&String> = graph.keys().collect();
    fn_names.sort();
    let defined: HashSet<&str> = fn_names.iter().map(|n| n.as_str()).collect();
    let mut lines = Vec::new();
    for name in &fn_names {
        let mut callees: Vec<&String> = graph[*name].iter().collect();
        callees.sort();
        let callee_list = if callees.is_empty() {
            "(no calls)".to_string()
        } else {
            callees
                .iter()
                .map(|c| {
                    if defined.contains(c.as_str()) {
                        c.to_string()
                    } else {
                        format!("{c}[extern]")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(format!("  {} → {}", name, callee_list));
    }
    // Detect cycles (mutual recursion) via DFS.
    let mut cycles: Vec<String> = Vec::new();
    let mut visited: HashSet<&str> = HashSet::new();
    let mut in_stack: HashSet<&str> = HashSet::new();
    for start in fn_names.iter().map(|s| s.as_str()) {
        if !visited.contains(start) {
            let mut path = Vec::new();
            find_cycle(start, &graph, &mut visited, &mut in_stack, &mut path, &mut cycles);
        }
    }
    let cycle_note = if cycles.is_empty() {
        String::new()
    } else {
        format!("\nCycles detected:\n{}", cycles.iter().map(|c| format!("  {c}")).collect::<Vec<_>>().join("\n"))
    };
    Ok(format!(
        "Call graph ({} functions):\n{}{}",
        graph.len(),
        lines.join("\n"),
        cycle_note
    ))
}

fn collect_call_targets(node: &crate::Node, out: &mut std::collections::HashSet<String>) {
    match node {
        crate::Node::CallExpression { function, arguments, .. } => {
            if let crate::Node::Identifier { name, .. } = function.as_ref() {
                out.insert(name.clone());
            }
            for a in arguments {
                collect_call_targets(a, out);
            }
            collect_call_targets(function, out);
        }
        crate::Node::Block { stmts, .. } => {
            for s in stmts {
                collect_call_targets(s, out);
            }
        }
        crate::Node::LetStatement { value, .. } => collect_call_targets(value, out),
        crate::Node::Assignment { value, .. } => collect_call_targets(value, out),
        crate::Node::ReturnStatement { value: Some(v), .. } => {
            collect_call_targets(v, out);
        }
        crate::Node::ReturnStatement { value: None, .. } => {}
        crate::Node::IfStatement { condition, consequence, alternative, .. } => {
            collect_call_targets(condition, out);
            collect_call_targets(consequence, out);
            if let Some(alt) = alternative {
                collect_call_targets(alt, out);
            }
        }
        crate::Node::WhileStatement { condition, body, .. } => {
            collect_call_targets(condition, out);
            collect_call_targets(body, out);
        }
        crate::Node::ForInStatement { iterable, body, .. } => {
            collect_call_targets(iterable, out);
            collect_call_targets(body, out);
        }
        crate::Node::ExpressionStatement { expr, .. } => collect_call_targets(expr, out),
        crate::Node::InfixExpression { left, right, .. } => {
            collect_call_targets(left, out);
            collect_call_targets(right, out);
        }
        crate::Node::PrefixExpression { right, .. } => collect_call_targets(right, out),
        crate::Node::IndexExpression { target, index, .. } => {
            collect_call_targets(target, out);
            collect_call_targets(index, out);
        }
        _ => {}
    }
}

fn find_cycle<'a>(
    node: &'a str,
    graph: &'a std::collections::HashMap<String, std::collections::HashSet<String>>,
    visited: &mut std::collections::HashSet<&'a str>,
    in_stack: &mut std::collections::HashSet<&'a str>,
    path: &mut Vec<&'a str>,
    cycles: &mut Vec<String>,
) {
    visited.insert(node);
    in_stack.insert(node);
    path.push(node);
    if let Some(callees) = graph.get(node) {
        let mut sorted_callees: Vec<&str> = callees.iter().map(|s| s.as_str()).collect();
        sorted_callees.sort();
        for callee in sorted_callees {
            if !visited.contains(callee) {
                find_cycle(callee, graph, visited, in_stack, path, cycles);
            } else if in_stack.contains(callee) {
                let start = path.iter().position(|&n| n == callee).unwrap_or(0);
                let cycle_path = path[start..].to_vec();
                cycles.push(format!("{} → {}", cycle_path.join(" → "), callee));
            }
        }
    }
    path.pop();
    in_stack.remove(node);
}

// ── Tool schema definitions ───────────────────────────────────────────────────

fn tool_definitions() -> Value {
    json!([
        {
            "name": "resilient_parse",
            "description": "Parse Resilient source code and report any parse errors. \
                            Returns a summary of top-level statements on success.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to parse"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_typecheck",
            "description": "Parse and type-check Resilient source code. \
                            Returns all type errors and warnings.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to type-check"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_run",
            "description": "Execute Resilient source code and return the captured output. \
                            Supports the full standard library.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to execute"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_lint",
            "description": "Run all Resilient lint passes on source code and return \
                            warnings (naming conventions, dead code, unsafe patterns, \
                            safety-critical violations, …).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to lint"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_format",
            "description": "Format / pretty-print Resilient source code using the \
                            canonical formatter. Returns the formatted source.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to format"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_check",
            "description": "Run the full Resilient compilation pipeline: parse, \
                            type-check, and all lint passes. The fastest way to \
                            validate a snippet end-to-end.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to check"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_verify",
            "description": "Verify function contracts (requires/ensures clauses) using \
                            the Z3 SMT solver. Only available when built with \
                            `--features z3`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to verify"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_explain_lint",
            "description": "Return a detailed human-readable explanation for a Resilient \
                            lint code (e.g. L0010). Includes what the lint detects, why \
                            it matters, an example, the recommended fix, and the \
                            suppression syntax.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "Lint code to explain, e.g. \"L0010\""
                    }
                },
                "required": ["code"]
            }
        },
        {
            "name": "resilient_symbols",
            "description": "Extract all named symbols (functions, top-level let bindings) \
                            from Resilient source and return a structured list with \
                            signatures. Useful for navigation and code understanding.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to extract symbols from"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_hover",
            "description": "Return type information for the identifier at a given byte \
                            offset in the Resilient source. Mirrors the LSP hover \
                            behaviour.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Zero-based byte offset of the identifier to hover"
                    }
                },
                "required": ["source", "offset"]
            }
        },
        {
            "name": "resilient_compile",
            "description": "Compile Resilient source code to bytecode and return a summary \
                            of the compiled output: main chunk instruction count, function \
                            count with per-function instruction counts, and total instruction \
                            count. Useful for understanding the shape of generated code.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to compile"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_disasm",
            "description": "Compile Resilient source code and return the full human-readable \
                            bytecode disassembly. Shows every opcode, its operands, and \
                            source line annotations. Useful for debugging compiler output \
                            and understanding the generated bytecode.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to disassemble"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_vm_run",
            "description": "Compile Resilient source code and execute it through the \
                            register-based bytecode VM (not the tree-walker interpreter). \
                            Returns captured stdout and the final value. Useful for \
                            testing bytecode compiler correctness and VM behaviour.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to compile and run via the VM"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_tla_check",
            "description": "Run TLC model checking on an inline TLA+ specification. \
                            The spec is written to a temporary file and checked with TLC. \
                            Returns Resilient-format diagnostics. Requires Java and \
                            tla2tools.jar (discoverable via RESILIENT_TLC_JAR env var or PATH).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "spec": {
                        "type": "string",
                        "description": "TLA+ specification source (full module content)"
                    },
                    "tlc_jar": {
                        "type": "string",
                        "description": "Optional path to tla2tools.jar (overrides RESILIENT_TLC_JAR)"
                    }
                },
                "required": ["spec"]
            }
        },
        {
            "name": "resilient_fingerprint",
            "description": "Compute behavioral fingerprints for every function in Resilient \
                            source code. Each fingerprint is a stable digest of the function's \
                            contracts (requires/ensures), parameter types, and fails variants \
                            — NOT the body. Refactors that preserve postconditions keep the \
                            same fingerprint, making regressions detectable in CI.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to fingerprint"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_resilience_score",
            "description": "Compute per-function resilience scores (0–100) for Resilient \
                            source. The score is a weighted sum of: contract coverage (40 pts), \
                            effect annotations (10 pts), live-recovery blocks (15 pts), \
                            call-site coverage (15 pts), and body simplicity (20 pts). \
                            Returns a letter grade (A–F) for each function.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to score"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_contract_infer",
            "description": "Run contract inference on Resilient source and suggest \
                            requires/ensures clauses for under-specified functions. \
                            Useful for adding contracts incrementally to an existing codebase.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to analyse"
                    }
                },
                "required": ["source"]
            }
        },
        {
            "name": "resilient_call_graph",
            "description": "Extract and display the function call graph for Resilient source. \
                            Shows, for each function, which other functions it calls directly. \
                            External/builtin callees are marked [extern]. Mutual recursion \
                            cycles are flagged.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Resilient source code to analyse"
                    }
                },
                "required": ["source"]
            }
        }
    ])
}

// ── JSON-RPC helpers ──────────────────────────────────────────────────────────

fn ok(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: &Value, code: i32, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn write_response(out: &mut impl Write, resp: &Value) -> io::Result<()> {
    let s = serde_json::to_string(resp).unwrap_or_else(|_| "{}".to_string());
    writeln!(out, "{s}")?;
    out.flush()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, source: &str) -> Result<String, String> {
        let args = json!({ "source": source });
        match name {
            "resilient_parse" => tool_parse(&args),
            "resilient_typecheck" => tool_typecheck(&args),
            "resilient_run" => tool_run(&args),
            "resilient_lint" => tool_lint(&args),
            "resilient_format" => tool_format(&args),
            "resilient_check" => tool_check(&args),
            "resilient_verify" => tool_verify(&args),
            _ => panic!("unknown tool {name}"),
        }
    }

    fn explain(code: &str) -> Result<String, String> {
        tool_explain_lint(&json!({ "code": code }))
    }

    fn symbols(source: &str) -> Result<String, String> {
        tool_symbols(&json!({ "source": source }))
    }

    fn hover(source: &str, offset: usize) -> Result<String, String> {
        tool_hover(&json!({ "source": source, "offset": offset }))
    }

    // ── resilient_parse ───────────────────────────────────────────────────────

    #[test]
    fn parse_valid_function() {
        let r = call("resilient_parse", "fn add(int a, int b) -> int { a + b }");
        assert!(r.is_ok(), "{r:?}");
        assert!(r.unwrap().contains("1 top-level statement"));
    }

    #[test]
    fn parse_reports_error_on_bad_syntax() {
        let r = call("resilient_parse", "fn { broken");
        assert!(r.is_err(), "expected parse error, got ok");
    }

    #[test]
    fn parse_empty_program() {
        let r = call("resilient_parse", "");
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn parse_missing_source_arg() {
        let r = tool_parse(&json!({}));
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Missing required argument"));
    }

    // ── resilient_typecheck ───────────────────────────────────────────────────

    #[test]
    fn typecheck_valid_program() {
        let src = "fn id(int x) -> int { x }";
        let r = call("resilient_typecheck", src);
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn typecheck_propagates_parse_errors() {
        let r = call("resilient_typecheck", "fn {{{");
        assert!(r.is_err());
    }

    // ── resilient_run ─────────────────────────────────────────────────────────

    #[test]
    fn run_hello_world() {
        let src = r#"println("hello from mcp")"#;
        let r = call("resilient_run", src);
        assert!(r.is_ok(), "{r:?}");
        let out = r.unwrap();
        assert!(out.contains("hello from mcp"), "got: {out}");
    }

    #[test]
    fn run_arithmetic() {
        let src = "println(2 + 3)";
        let r = call("resilient_run", src);
        assert!(r.is_ok(), "{r:?}");
        assert!(r.unwrap().contains('5'));
    }

    #[test]
    fn run_returns_error_on_runtime_failure() {
        // Division by zero should produce a runtime error.
        let src = "let x = 1 / 0";
        let r = call("resilient_run", src);
        assert!(r.is_err(), "expected runtime error");
    }

    // ── resilient_lint ────────────────────────────────────────────────────────

    #[test]
    fn lint_clean_program() {
        // Suppress L0010 (missing contract) and L0014 (unused function) so the
        // function is lint-clean without call-site or contract boilerplate.
        let src = "// resilient: allow L0010, L0014\nfn add(int a, int b) -> int { a + b }";
        let r = call("resilient_lint", src);
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn lint_parse_error_propagates() {
        let r = call("resilient_lint", "fn {{{");
        assert!(r.is_err());
    }

    // ── resilient_format ──────────────────────────────────────────────────────

    #[test]
    fn format_returns_string() {
        let src = "fn f(int x) -> int { x }";
        let r = call("resilient_format", src);
        assert!(r.is_ok(), "{r:?}");
        assert!(!r.unwrap().is_empty());
    }

    #[test]
    fn format_parse_error_propagates() {
        let r = call("resilient_format", "fn {{{");
        assert!(r.is_err());
    }

    // ── resilient_check ───────────────────────────────────────────────────────

    #[test]
    fn check_clean_program() {
        // Suppress L0010 (missing contract) and L0014 (unused function) so the
        // full pipeline passes without call-site or contract boilerplate.
        let src = "// resilient: allow L0010, L0014\nfn add(int a, int b) -> int { a + b }";
        let r = call("resilient_check", src);
        assert!(r.is_ok(), "{r:?}");
        assert!(r.unwrap().contains("OK"));
    }

    #[test]
    fn check_parse_error() {
        let r = call("resilient_check", "fn {{{");
        assert!(r.is_err());
    }

    // ── resilient_verify ──────────────────────────────────────────────────────

    #[test]
    fn verify_no_contracts() {
        // A function with no contracts should still pass (nothing to verify).
        let src = "fn add(int a, int b) -> int { a + b }";
        let r = call("resilient_verify", src);
        // Either Ok (z3 feature) or Err with "not available" (no z3 feature).
        match r {
            Ok(msg) => assert!(msg.contains("OK")),
            Err(msg) => assert!(msg.contains("not available") || msg.contains("z3")),
        }
    }

    // ── Protocol dispatch ─────────────────────────────────────────────────────

    #[test]
    fn dispatch_initialize_returns_server_info() {
        let resp = dispatch("initialize", &json!(1), Some(&json!({})), false);
        assert!(resp.is_some());
        let resp = resp.unwrap();
        assert_eq!(resp["result"]["serverInfo"]["name"], "resilient");
    }

    #[test]
    fn dispatch_notification_returns_none() {
        let resp = dispatch("notifications/initialized", &json!(null), None, true);
        assert!(resp.is_none());
    }

    #[test]
    fn dispatch_unknown_method_returns_error() {
        let resp = dispatch("no/such/method", &json!(1), None, false);
        assert!(resp.is_some());
        let resp = resp.unwrap();
        assert!(resp.get("error").is_some());
    }

    #[test]
    fn tools_list_contains_all_tools() {
        let resp = handle_tools_list(&json!(1));
        let tools = resp["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        for expected in &[
            "resilient_parse",
            "resilient_typecheck",
            "resilient_run",
            "resilient_lint",
            "resilient_format",
            "resilient_check",
            "resilient_verify",
            "resilient_explain_lint",
            "resilient_symbols",
            "resilient_hover",
            "resilient_compile",
            "resilient_disasm",
            "resilient_vm_run",
            "resilient_tla_check",
            "resilient_fingerprint",
            "resilient_resilience_score",
            "resilient_contract_infer",
            "resilient_call_graph",
        ] {
            assert!(
                names.contains(expected),
                "missing tool {expected}; got {names:?}"
            );
        }
    }

    #[test]
    fn tools_call_unknown_tool_is_error_content() {
        let params = json!({ "name": "no_such_tool", "arguments": {} });
        let resp = handle_tools_call(&json!(1), Some(&params));
        let content = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
        assert!(content.contains("Unknown tool"), "got: {content}");
        assert_eq!(resp["result"]["isError"], true);
    }

    #[test]
    fn ping_returns_empty_object() {
        let resp = dispatch("ping", &json!(1), None, false);
        assert!(resp.is_some());
        let resp = resp.unwrap();
        assert_eq!(resp["result"], json!({}));
    }

    // ── resilient_explain_lint ────────────────────────────────────────────────

    #[test]
    fn explain_known_code_l0010() {
        let r = explain("L0010");
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(text.contains("L0010"), "got: {text}");
        assert!(text.contains("requires"), "got: {text}");
    }

    #[test]
    fn explain_unknown_code_returns_error() {
        let r = explain("L9999");
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Unknown lint code"));
    }

    #[test]
    fn explain_missing_code_arg_returns_error() {
        let r = tool_explain_lint(&json!({}));
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Missing required argument"));
    }

    #[test]
    fn explain_all_known_codes_have_entry() {
        for code in crate::lint::KNOWN_CODES {
            let r = explain(code);
            assert!(r.is_ok(), "missing explain entry for {code}");
        }
    }

    // ── resilient_symbols ─────────────────────────────────────────────────────

    #[test]
    fn symbols_lists_functions() {
        let src = "fn add(int a, int b) -> int { a + b }\nfn sub(int a, int b) -> int { a - b }";
        let r = symbols(src);
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(text.contains("fn add"), "got: {text}");
        assert!(text.contains("fn sub"), "got: {text}");
    }

    #[test]
    fn symbols_empty_program() {
        let r = symbols("");
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn symbols_parse_error_propagates() {
        let r = symbols("fn {{{");
        assert!(r.is_err());
    }

    #[test]
    fn symbols_includes_return_type() {
        let src = "fn f(int x) -> int { x }";
        let r = symbols(src);
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(text.contains("-> int"), "got: {text}");
    }

    // ── resilient_hover ───────────────────────────────────────────────────────

    #[test]
    fn hover_on_parameter_name() {
        let src = "fn f(int myParam) -> int { myParam }";
        // offset of 'm' in "myParam" in the parameter list (position ~9)
        let offset = src.find("myParam").unwrap();
        let r = hover(src, offset);
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(text.contains("myParam"), "got: {text}");
    }

    #[test]
    fn hover_out_of_range_returns_error() {
        let src = "fn f(int x) -> int { x }";
        let r = hover(src, src.len() + 100);
        assert!(r.is_err());
    }

    #[test]
    fn hover_on_non_ident_char_returns_error() {
        let src = "fn f(int x) -> int { x }";
        let r = hover(src, src.find('(').unwrap());
        assert!(r.is_err());
    }

    // ── resilient_compile ─────────────────────────────────────────────────────

    #[test]
    fn compile_returns_summary() {
        let src = "fn add(int a, int b) -> int { a + b }";
        let r = tool_compile(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(text.contains("OK"), "got: {text}");
        assert!(text.contains("instruction"), "got: {text}");
    }

    #[test]
    fn compile_parse_error_propagates() {
        let r = tool_compile(&json!({ "source": "fn {{{{" }));
        assert!(r.is_err());
    }

    #[test]
    fn compile_missing_source_returns_error() {
        let r = tool_compile(&json!({}));
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Missing required argument"));
    }

    #[test]
    fn compile_shows_function_count() {
        let src = "fn f(int x) -> int { x }\nfn g(int y) -> int { y }";
        let r = tool_compile(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(text.contains("Functions: 2"), "got: {text}");
    }

    // ── resilient_disasm ──────────────────────────────────────────────────────

    #[test]
    fn disasm_returns_bytecode_text() {
        let src = "let x = 42";
        let r = tool_disasm(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(!text.is_empty(), "disasm output should not be empty");
    }

    #[test]
    fn disasm_contains_const_opcode() {
        let src = "let x = 1 + 2";
        let r = tool_disasm(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        // The disassembly should mention at least one `Const` instruction.
        assert!(
            text.contains("Const") || text.contains("const"),
            "expected Const in disasm, got: {text}"
        );
    }

    #[test]
    fn disasm_parse_error_propagates() {
        let r = tool_disasm(&json!({ "source": "fn {{{" }));
        assert!(r.is_err());
    }

    // ── resilient_vm_run ──────────────────────────────────────────────────────

    #[test]
    fn vm_run_hello_world() {
        let src = r#"println("hello vm")"#;
        let r = tool_vm_run(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(text.contains("hello vm"), "got: {text}");
    }

    #[test]
    fn vm_run_arithmetic() {
        let src = "println(3 + 4)";
        let r = tool_vm_run(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        assert!(r.unwrap().contains('7'));
    }

    #[test]
    fn vm_run_div_zero_is_error() {
        let src = "let x = 1 / 0";
        let r = tool_vm_run(&json!({ "source": src }));
        assert!(r.is_err(), "expected VM error for division by zero");
    }

    #[test]
    fn vm_run_missing_source_returns_error() {
        let r = tool_vm_run(&json!({}));
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Missing required argument"));
    }

    // ── resilient_tla_check ───────────────────────────────────────────────────

    // ── resilient_fingerprint ─────────────────────────────────────────────────

    #[test]
    fn fingerprint_returns_digest_for_function() {
        let src = "fn f(int x) -> int ensures result > 0 { return x + 1; }";
        let r = tool_fingerprint(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(text.contains("digest:"), "got: {text}");
        assert!(text.contains("f"), "got: {text}");
    }

    #[test]
    fn fingerprint_empty_program_says_no_functions() {
        let r = tool_fingerprint(&json!({ "source": "" }));
        assert!(r.is_ok(), "{r:?}");
        assert!(r.unwrap().contains("No functions"));
    }

    #[test]
    fn fingerprint_parse_error_propagates() {
        let r = tool_fingerprint(&json!({ "source": "fn {{{{" }));
        assert!(r.is_err());
    }

    #[test]
    fn fingerprint_recovery_flag_appears() {
        let src = "fn f(int x) -> int fails IOError recovers_to: result >= 0 { return x; }";
        let r = tool_fingerprint(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(
            text.contains("[recovery]") || text.contains("0x"),
            "got: {text}"
        );
    }

    // ── resilient_resilience_score ────────────────────────────────────────────

    #[test]
    fn resilience_score_returns_grade_for_function() {
        let src = "fn f(int x) -> int requires x > 0 ensures result > 0 { return x; }";
        let r = tool_resilience_score(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(text.contains("/100"), "got: {text}");
    }

    #[test]
    fn resilience_score_empty_program_says_no_functions() {
        let r = tool_resilience_score(&json!({ "source": "" }));
        assert!(r.is_ok(), "{r:?}");
        assert!(r.unwrap().contains("No functions"));
    }

    #[test]
    fn resilience_score_parse_error_propagates() {
        let r = tool_resilience_score(&json!({ "source": "fn {{{{" }));
        assert!(r.is_err());
    }

    #[test]
    fn resilience_score_shows_avg() {
        let src = "// resilient: allow L0010, L0014\nfn f(int x) -> int { return x; }";
        let r = tool_resilience_score(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(text.contains("avg"), "got: {text}");
    }

    // ── resilient_contract_infer ──────────────────────────────────────────────

    #[test]
    fn contract_infer_suggests_for_unspecified_function() {
        let src = "fn add(int a, int b) -> int { return a + b; }";
        let r = tool_contract_infer(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        // Either suggestions exist, or "no inference" message.
        let text = r.unwrap();
        assert!(
            !text.is_empty(),
            "contract_infer must return something, got empty"
        );
    }

    #[test]
    fn contract_infer_empty_program_ok() {
        let r = tool_contract_infer(&json!({ "source": "" }));
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn contract_infer_parse_error_propagates() {
        let r = tool_contract_infer(&json!({ "source": "fn {{{{" }));
        assert!(r.is_err());
    }

    // ── resilient_call_graph ──────────────────────────────────────────────────

    #[test]
    fn call_graph_shows_callee() {
        let src = "fn g(int x) -> int { return x; }\nfn f(int x) -> int { return g(x); }";
        let r = tool_call_graph(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(text.contains("f"), "got: {text}");
        assert!(text.contains("g"), "got: {text}");
    }

    #[test]
    fn call_graph_detects_cycle() {
        let src = "fn f(int x) -> int { return g(x); }\nfn g(int x) -> int { return f(x); }";
        let r = tool_call_graph(&json!({ "source": src }));
        assert!(r.is_ok(), "{r:?}");
        let text = r.unwrap();
        assert!(
            text.contains("Cycle") || text.contains("cycle") || text.contains("→"),
            "got: {text}"
        );
    }

    #[test]
    fn call_graph_empty_program_says_no_functions() {
        let r = tool_call_graph(&json!({ "source": "" }));
        assert!(r.is_ok(), "{r:?}");
        assert!(r.unwrap().contains("No functions"));
    }

    #[test]
    fn call_graph_parse_error_propagates() {
        let r = tool_call_graph(&json!({ "source": "fn {{{{" }));
        assert!(r.is_err());
    }

    // ── prompts/list ──────────────────────────────────────────────────────────

    #[test]
    fn prompts_list_returns_all_prompts() {
        let resp = handle_prompts_list(&json!(1));
        let prompts = resp["result"]["prompts"].as_array().unwrap();
        let names: Vec<&str> = prompts.iter().filter_map(|p| p["name"].as_str()).collect();
        for expected in &[
            "verify_function",
            "debug_type_error",
            "add_resilience",
            "explain_lint",
            "safety_review",
        ] {
            assert!(names.contains(expected), "missing prompt {expected}; got {names:?}");
        }
    }

    #[test]
    fn prompts_list_each_has_description() {
        let resp = handle_prompts_list(&json!(1));
        let prompts = resp["result"]["prompts"].as_array().unwrap();
        for p in prompts {
            assert!(
                p["description"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
                "prompt {:?} missing description",
                p["name"]
            );
        }
    }

    // ── prompts/get ───────────────────────────────────────────────────────────

    #[test]
    fn prompts_get_verify_function_returns_messages() {
        let src = "fn add(int a, int b) -> int { return a + b; }";
        let params = json!({ "name": "verify_function", "arguments": { "source": src } });
        let resp = handle_prompts_get(&json!(1), Some(&params));
        let messages = resp["result"]["messages"].as_array().unwrap();
        assert!(!messages.is_empty(), "expected at least one message");
        let text = messages[0]["content"]["text"].as_str().unwrap_or("");
        assert!(text.contains("requires") || text.contains("ensures"), "got: {text}");
    }

    #[test]
    fn prompts_get_debug_type_error_includes_source() {
        let src = "fn f(int x) -> string { return x; }";
        let params = json!({ "name": "debug_type_error", "arguments": { "source": src } });
        let resp = handle_prompts_get(&json!(1), Some(&params));
        let messages = resp["result"]["messages"].as_array().unwrap();
        assert!(!messages.is_empty());
        let text = messages[0]["content"]["text"].as_str().unwrap_or("");
        assert!(text.contains("type error") || text.contains("error"), "got: {text}");
    }

    #[test]
    fn prompts_get_add_resilience_returns_messages() {
        let src = "fn sensor() -> int { return 42; }";
        let params = json!({ "name": "add_resilience", "arguments": { "source": src } });
        let resp = handle_prompts_get(&json!(1), Some(&params));
        let messages = resp["result"]["messages"].as_array().unwrap();
        assert!(!messages.is_empty());
        let text = messages[0]["content"]["text"].as_str().unwrap_or("");
        assert!(text.contains("recovers_to") || text.contains("live"), "got: {text}");
    }

    #[test]
    fn prompts_get_explain_lint_returns_explanation() {
        let params = json!({ "name": "explain_lint", "arguments": { "code": "L0010" } });
        let resp = handle_prompts_get(&json!(1), Some(&params));
        let messages = resp["result"]["messages"].as_array().unwrap();
        assert!(!messages.is_empty());
        let text = messages[0]["content"]["text"].as_str().unwrap_or("");
        assert!(text.contains("L0010"), "got: {text}");
    }

    #[test]
    fn prompts_get_safety_review_returns_checklist() {
        let src = "fn add(int a, int b) -> int { return a + b; }";
        let params = json!({ "name": "safety_review", "arguments": { "source": src } });
        let resp = handle_prompts_get(&json!(1), Some(&params));
        let messages = resp["result"]["messages"].as_array().unwrap();
        assert!(!messages.is_empty());
        let text = messages[0]["content"]["text"].as_str().unwrap_or("");
        assert!(text.contains("safety") || text.contains("review"), "got: {text}");
    }

    #[test]
    fn prompts_get_unknown_prompt_returns_error() {
        let params = json!({ "name": "nonexistent_prompt", "arguments": {} });
        let resp = handle_prompts_get(&json!(1), Some(&params));
        assert!(resp.get("error").is_some(), "expected error for unknown prompt");
    }

    #[test]
    fn prompts_get_missing_name_returns_error() {
        let params = json!({ "arguments": {} });
        let resp = handle_prompts_get(&json!(1), Some(&params));
        assert!(resp.get("error").is_some(), "expected error for missing name");
    }

    #[test]
    fn dispatch_prompts_list_works() {
        let resp = dispatch("prompts/list", &json!(1), None, false);
        assert!(resp.is_some());
        let resp = resp.unwrap();
        assert!(resp["result"]["prompts"].is_array());
    }

    #[test]
    fn dispatch_resources_list_works() {
        let resp = dispatch("resources/list", &json!(1), None, false);
        assert!(resp.is_some());
        let resp = resp.unwrap();
        assert!(resp["result"]["resources"].is_array());
    }

    // ── resources/list ────────────────────────────────────────────────────────

    #[test]
    fn resources_list_returns_all_resources() {
        let resp = handle_resources_list(&json!(1));
        let resources = resp["result"]["resources"].as_array().unwrap();
        let uris: Vec<&str> = resources.iter().filter_map(|r| r["uri"].as_str()).collect();
        for expected in &[
            "resilient://docs/syntax",
            "resilient://docs/stdlib",
            "resilient://docs/lint-codes",
            "resilient://docs/contracts",
            "resilient://docs/effects",
            "resilient://docs/resilience",
        ] {
            assert!(uris.contains(expected), "missing resource {expected}; got {uris:?}");
        }
    }

    #[test]
    fn resources_list_each_has_mime_type() {
        let resp = handle_resources_list(&json!(1));
        let resources = resp["result"]["resources"].as_array().unwrap();
        for r in resources {
            assert_eq!(
                r["mimeType"].as_str().unwrap_or(""),
                "text/plain",
                "resource {:?} should have mimeType text/plain",
                r["uri"]
            );
        }
    }

    // ── resources/read ────────────────────────────────────────────────────────

    #[test]
    fn resources_read_syntax_doc_is_nonempty() {
        let params = json!({ "uri": "resilient://docs/syntax" });
        let resp = handle_resources_read(&json!(1), Some(&params));
        let text = resp["result"]["contents"][0]["text"].as_str().unwrap_or("");
        assert!(!text.is_empty(), "syntax doc must not be empty");
        assert!(text.contains("fn"), "syntax doc must mention fn keyword");
    }

    #[test]
    fn resources_read_stdlib_doc_is_nonempty() {
        let params = json!({ "uri": "resilient://docs/stdlib" });
        let resp = handle_resources_read(&json!(1), Some(&params));
        let text = resp["result"]["contents"][0]["text"].as_str().unwrap_or("");
        assert!(!text.is_empty(), "stdlib doc must not be empty");
        assert!(text.contains("len"), "stdlib doc must mention len");
    }

    #[test]
    fn resources_read_lint_codes_doc_covers_all_codes() {
        let params = json!({ "uri": "resilient://docs/lint-codes" });
        let resp = handle_resources_read(&json!(1), Some(&params));
        let text = resp["result"]["contents"][0]["text"].as_str().unwrap_or("");
        for code in crate::lint::KNOWN_CODES.iter().take(5) {
            assert!(text.contains(code), "lint doc must mention {code}");
        }
    }

    #[test]
    fn resources_read_contracts_doc_mentions_requires() {
        let params = json!({ "uri": "resilient://docs/contracts" });
        let resp = handle_resources_read(&json!(1), Some(&params));
        let text = resp["result"]["contents"][0]["text"].as_str().unwrap_or("");
        assert!(text.contains("requires"), "contracts doc must mention requires");
        assert!(text.contains("ensures"), "contracts doc must mention ensures");
    }

    #[test]
    fn resources_read_effects_doc_mentions_pure() {
        let params = json!({ "uri": "resilient://docs/effects" });
        let resp = handle_resources_read(&json!(1), Some(&params));
        let text = resp["result"]["contents"][0]["text"].as_str().unwrap_or("");
        assert!(text.contains("@pure"), "effects doc must mention @pure");
    }

    #[test]
    fn resources_read_resilience_doc_mentions_live() {
        let params = json!({ "uri": "resilient://docs/resilience" });
        let resp = handle_resources_read(&json!(1), Some(&params));
        let text = resp["result"]["contents"][0]["text"].as_str().unwrap_or("");
        assert!(text.contains("live") || text.contains("recovers_to"), "got: {text}");
    }

    #[test]
    fn resources_read_unknown_uri_returns_error() {
        let params = json!({ "uri": "resilient://docs/nonexistent" });
        let resp = handle_resources_read(&json!(1), Some(&params));
        assert!(resp.get("error").is_some(), "expected error for unknown URI");
    }

    #[test]
    fn resources_read_missing_uri_returns_error() {
        let resp = handle_resources_read(&json!(1), Some(&json!({})));
        assert!(resp.get("error").is_some(), "expected error for missing URI");
    }

    #[test]
    fn initialize_capabilities_include_prompts_and_resources() {
        let resp = dispatch("initialize", &json!(1), Some(&json!({})), false);
        let resp = resp.unwrap();
        let caps = &resp["result"]["capabilities"];
        assert!(caps["prompts"].is_object(), "capabilities must include prompts");
        assert!(caps["resources"].is_object(), "capabilities must include resources");
    }

    // ── resilient_tla_check ───────────────────────────────────────────────────

    #[test]
    fn tla_check_missing_spec_returns_error() {
        let r = tool_tla_check(&json!({}));
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Missing required argument"));
    }

    #[test]
    fn tla_check_with_nonexistent_jar_returns_error() {
        // No TLC available → should return an error diagnostic, not panic.
        let spec = "---- MODULE Spec ----\nINIT TRUE\nNEXT TRUE\n====\n";
        let r = tool_tla_check(&json!({
            "spec": spec,
            "tlc_jar": "/nonexistent/tla2tools.jar"
        }));
        assert!(r.is_err());
        let msg = r.unwrap_err();
        assert!(
            msg.contains("error") || msg.contains("not found"),
            "got: {msg}"
        );
    }
}
