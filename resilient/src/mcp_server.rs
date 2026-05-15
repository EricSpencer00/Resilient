//! MCP server for the Resilient compiler.
//!
//! Implements the [Model Context Protocol](https://modelcontextprotocol.io)
//! over stdio (newline-delimited JSON-RPC 2.0), exposing the Resilient
//! compiler pipeline as tools that AI assistants can invoke directly.
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
                "tools": {}
            },
            "serverInfo": {
                "name": "resilient",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )
}

fn handle_tools_list(id: &Value) -> Value {
    ok(id, json!({ "tools": tool_definitions() }))
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
        other => Err(format!("Unknown tool: {other}")),
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
}
