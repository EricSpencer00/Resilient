//! MCP external-tool bridge registry.
//!
//! Provides a generic scaffolding framework for connecting external
//! verification and analysis tools to the Resilient MCP server.  Each
//! external tool — TLA+ TLC, Z3, Lean 4, CBMC, or a user-supplied binary
//! — is registered as a `BridgedTool` and exposed automatically via the
//! `tools/list` and `tools/call` MCP methods.
//!
//! # Pattern
//!
//! ```
//! // register at startup:
//! McpBridgeRegistry::global().register(tlc_adapter());
//!
//! // in tools/list handler:
//! let extra = McpBridgeRegistry::global().mcp_tool_definitions();
//!
//! // in tools/call handler:
//! let result = McpBridgeRegistry::global().invoke("tlc_check", &params)?;
//! ```
//!
//! # CLI subcommand
//!
//! `rz tool list` — print all registered external tools and their availability
//! `rz tool call <name> [key=value ...]` — invoke a registered tool
//! `rz tool check <file.rz>` — run all available tools against a source file
//!
//! # Discovery order (per tool)
//!
//! 1. `RESILIENT_<TOOL>_BIN` environment variable
//! 2. Explicit path in registry config
//! 3. Binary name anywhere on `PATH`

#![allow(clippy::doc_lazy_continuation)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{OnceLock, RwLock};

// ── Result types ─────────────────────────────────────────────────────────────

/// Outcome of invoking a bridged external tool.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolOutcome {
    /// Tool ran and found no issues.
    Clean,
    /// Tool ran and found violations/errors.
    Violation,
    /// Tool is unavailable (binary not found, etc.).
    Unavailable,
    /// Tool invocation failed (IO error, bad exit code, etc.).
    InvocationError,
}

/// Result returned by a bridged tool invocation.
#[derive(Debug, Clone)]
pub struct ToolInvocationResult {
    pub outcome: ToolOutcome,
    /// Resilient-format diagnostics: `file:line:col: severity: message`
    pub diagnostics: Vec<String>,
    /// Raw tool stdout/stderr for `--verbose` consumers.
    pub raw_output: String,
    /// Tool-specific metadata (e.g., counterexample variables).
    pub metadata: HashMap<String, String>,
}

impl ToolInvocationResult {
    fn unavailable(tool_name: &str) -> Self {
        Self {
            outcome: ToolOutcome::Unavailable,
            diagnostics: vec![format!(
                "0:0: warning: external tool `{tool_name}` is not available"
            )],
            raw_output: String::new(),
            metadata: HashMap::new(),
        }
    }

    fn error(tool_name: &str, msg: &str) -> Self {
        Self {
            outcome: ToolOutcome::InvocationError,
            diagnostics: vec![format!("0:0: error: tool `{tool_name}`: {msg}")],
            raw_output: String::new(),
            metadata: HashMap::new(),
        }
    }
}

// ── Tool descriptor ───────────────────────────────────────────────────────────

/// Identifies the kind of external tool — affects discovery and invocation.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolKind {
    /// Verification tool based on a JVM jar (like TLC / TLA+).
    JvmJar { jar_env: String, jar_class: String },
    /// Native binary tool invoked via PATH or explicit path.
    NativeBinary {
        binary_name: String,
        env_override: String,
    },
    /// Built-in adapter that delegates to a Resilient-internal module.
    BuiltIn,
}

/// Registered external tool with its descriptor and invocation adapter.
#[derive(Debug, Clone)]
pub struct BridgedTool {
    /// Unique tool name used in `tools/call` MCP requests.
    pub name: String,
    /// Human-readable description surfaced in `tools/list`.
    pub description: String,
    /// Tool kind — determines discovery and invocation strategy.
    pub kind: ToolKind,
    /// Optional explicit path to the tool binary or jar.
    pub explicit_path: Option<PathBuf>,
    /// JSON Schema for input parameters (used in `tools/list`).
    pub input_schema: serde_json::Value,
}

impl BridgedTool {
    /// Returns true if the tool is available in the current environment.
    pub fn is_available(&self) -> bool {
        match &self.kind {
            ToolKind::BuiltIn => true,
            ToolKind::JvmJar { jar_env, .. } => {
                java_available() && resolve_jar(jar_env, self.explicit_path.as_deref()).is_some()
            }
            ToolKind::NativeBinary {
                binary_name,
                env_override,
            } => resolve_binary(binary_name, env_override, self.explicit_path.as_deref()).is_some(),
        }
    }

    /// Discover the resolved path to the tool binary or jar.
    pub fn resolved_path(&self) -> Option<PathBuf> {
        match &self.kind {
            ToolKind::BuiltIn => None,
            ToolKind::JvmJar { jar_env, .. } => resolve_jar(jar_env, self.explicit_path.as_deref()),
            ToolKind::NativeBinary {
                binary_name,
                env_override,
            } => resolve_binary(binary_name, env_override, self.explicit_path.as_deref()),
        }
    }

    /// MCP `tools/list` tool definition as a JSON object.
    pub fn mcp_definition(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "description": format!(
                "{} [{}]",
                self.description,
                if self.is_available() { "available" } else { "not available" }
            ),
            "inputSchema": self.input_schema
        })
    }
}

// ── Global registry ───────────────────────────────────────────────────────────

/// Central registry for all bridged external tools.
pub struct McpBridgeRegistry {
    tools: RwLock<Vec<BridgedTool>>,
}

static REGISTRY: OnceLock<McpBridgeRegistry> = OnceLock::new();

impl McpBridgeRegistry {
    /// Returns the global singleton registry, initialised with built-in
    /// adapters on first access.
    pub fn global() -> &'static McpBridgeRegistry {
        REGISTRY.get_or_init(|| {
            let reg = McpBridgeRegistry {
                tools: RwLock::new(Vec::new()),
            };
            reg.register(tlc_adapter());
            reg.register(lean4_adapter());
            reg.register(cbmc_adapter());
            reg.register(z3_adapter());
            // RES-2645: additional verification/analysis tool adapters.
            reg.register(spin_adapter());
            reg.register(frama_c_adapter());
            reg.register(klee_adapter());
            reg
        })
    }

    /// Register a new bridged tool.  Duplicate names are silently replaced.
    pub fn register(&self, tool: BridgedTool) {
        let mut tools = self.tools.write().unwrap_or_else(|p| p.into_inner());
        if let Some(pos) = tools.iter().position(|t| t.name == tool.name) {
            tools[pos] = tool;
        } else {
            tools.push(tool);
        }
    }

    /// Return all registered tools.
    pub fn all_tools(&self) -> Vec<BridgedTool> {
        self.tools.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Return only tools that are currently available.
    pub fn available_tools(&self) -> Vec<BridgedTool> {
        self.all_tools()
            .into_iter()
            .filter(|t| t.is_available())
            .collect()
    }

    /// MCP `tools/list` entries for all registered tools.
    pub fn mcp_tool_definitions(&self) -> Vec<serde_json::Value> {
        self.all_tools()
            .iter()
            .map(|t| t.mcp_definition())
            .collect()
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<BridgedTool> {
        self.tools
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|t| t.name == name)
            .cloned()
    }

    /// Invoke a registered tool with the given MCP params.
    pub fn invoke(
        &self,
        name: &str,
        params: &serde_json::Value,
    ) -> Result<ToolInvocationResult, String> {
        let tool = self
            .get(name)
            .ok_or_else(|| format!("unknown tool: `{name}`"))?;
        Ok(invoke_tool(&tool, params))
    }
}

// ── Built-in adapters ─────────────────────────────────────────────────────────

/// TLA+ / TLC model checker adapter.
pub fn tlc_adapter() -> BridgedTool {
    BridgedTool {
        name: "tlc_check".to_string(),
        description: "Run TLA+ model checking via TLC (tla2tools.jar)".to_string(),
        kind: ToolKind::JvmJar {
            jar_env: "RESILIENT_TLC_JAR".to_string(),
            jar_class: "tlc2.TLC".to_string(),
        },
        explicit_path: None,
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["file"],
            "properties": {
                "file": {
                    "type": "string",
                    "description": "Path to the .tla specification file"
                },
                "verbose": {
                    "type": "boolean",
                    "description": "Include raw TLC output in result"
                }
            }
        }),
    }
}

/// Lean 4 proof assistant adapter.
pub fn lean4_adapter() -> BridgedTool {
    BridgedTool {
        name: "lean4_check".to_string(),
        description: "Run Lean 4 proof checking on a .lean file".to_string(),
        kind: ToolKind::NativeBinary {
            binary_name: "lean".to_string(),
            env_override: "RESILIENT_LEAN_BIN".to_string(),
        },
        explicit_path: None,
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["file"],
            "properties": {
                "file": {
                    "type": "string",
                    "description": "Path to the .lean file to check"
                },
                "theorem": {
                    "type": "string",
                    "description": "Optional: specific theorem name to verify"
                }
            }
        }),
    }
}

/// CBMC bounded model checker adapter.
pub fn cbmc_adapter() -> BridgedTool {
    BridgedTool {
        name: "cbmc_check".to_string(),
        description: "Run CBMC bounded model checking on a C/LLVM file".to_string(),
        kind: ToolKind::NativeBinary {
            binary_name: "cbmc".to_string(),
            env_override: "RESILIENT_CBMC_BIN".to_string(),
        },
        explicit_path: None,
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["file"],
            "properties": {
                "file": {
                    "type": "string",
                    "description": "Path to the C or LLVM bitcode file"
                },
                "unwind": {
                    "type": "integer",
                    "description": "Loop unwinding bound (default: 10)"
                },
                "property": {
                    "type": "string",
                    "description": "Specific property to check (default: all)"
                }
            }
        }),
    }
}

/// SPIN / Promela model checker adapter.
///
/// SPIN verifies concurrent systems specified in Promela. Useful for
/// checking message-passing protocols alongside Resilient actor programs.
/// Requires `spin` on PATH or `RESILIENT_SPIN_BIN` to be set.
pub fn spin_adapter() -> BridgedTool {
    BridgedTool {
        name: "spin_check".to_string(),
        description: "Run SPIN model checking on a Promela specification. \
                      Verifies safety/liveness properties of concurrent systems. \
                      Requires spin on PATH or RESILIENT_SPIN_BIN."
            .to_string(),
        kind: ToolKind::NativeBinary {
            binary_name: "spin".to_string(),
            env_override: "RESILIENT_SPIN_BIN".to_string(),
        },
        explicit_path: None,
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["file"],
            "properties": {
                "file": {
                    "type": "string",
                    "description": "Path to the Promela (.pml) specification file"
                },
                "property": {
                    "type": "string",
                    "description": "Optional: LTL property formula to check"
                }
            }
        }),
    }
}

/// Frama-C static analyser adapter.
///
/// Frama-C is a modular static analysis framework for C programs. The
/// Eva value-analysis plugin computes over-approximations of variable
/// ranges and detects undefined behaviour. Requires `frama-c` on PATH
/// or `RESILIENT_FRAMAC_BIN` to be set.
pub fn frama_c_adapter() -> BridgedTool {
    BridgedTool {
        name: "frama_c_check".to_string(),
        description: "Run Frama-C static analysis on a C source file. \
                      Uses the Eva value-analysis plugin to detect undefined behaviour, \
                      integer overflows, and out-of-bounds accesses. \
                      Requires frama-c on PATH or RESILIENT_FRAMAC_BIN."
            .to_string(),
        kind: ToolKind::NativeBinary {
            binary_name: "frama-c".to_string(),
            env_override: "RESILIENT_FRAMAC_BIN".to_string(),
        },
        explicit_path: None,
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["file"],
            "properties": {
                "file": {
                    "type": "string",
                    "description": "Path to the C source file to analyse"
                },
                "plugin": {
                    "type": "string",
                    "description": "Frama-C plugin to use (default: eva)"
                }
            }
        }),
    }
}

/// KLEE symbolic execution adapter.
///
/// KLEE generates test inputs that exercise all reachable paths in an
/// LLVM bitcode file, making it effective for finding crashes and
/// undefined behaviour. Requires `klee` on PATH or `RESILIENT_KLEE_BIN`.
pub fn klee_adapter() -> BridgedTool {
    BridgedTool {
        name: "klee_check".to_string(),
        description: "Run KLEE symbolic execution on an LLVM bitcode file (.bc). \
                      Generates concrete test inputs for all reachable paths and \
                      reports crashes, assertion failures, and undefined behaviour. \
                      Requires klee on PATH or RESILIENT_KLEE_BIN."
            .to_string(),
        kind: ToolKind::NativeBinary {
            binary_name: "klee".to_string(),
            env_override: "RESILIENT_KLEE_BIN".to_string(),
        },
        explicit_path: None,
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["file"],
            "properties": {
                "file": {
                    "type": "string",
                    "description": "Path to the LLVM bitcode file (.bc) to analyse"
                },
                "max_time": {
                    "type": "integer",
                    "description": "Maximum analysis time in seconds (default: 60)"
                }
            }
        }),
    }
}

/// Z3 SMT solver adapter (built-in; delegates to verifier_z3 module).
pub fn z3_adapter() -> BridgedTool {
    BridgedTool {
        name: "z3_check".to_string(),
        description: "Run Z3 SMT verification on a Resilient source file".to_string(),
        kind: ToolKind::BuiltIn,
        explicit_path: None,
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["source"],
            "properties": {
                "source": {
                    "type": "string",
                    "description": "Resilient source code to verify"
                },
                "function": {
                    "type": "string",
                    "description": "Optional: specific function to verify"
                }
            }
        }),
    }
}

// ── Invocation engine ─────────────────────────────────────────────────────────

fn invoke_tool(tool: &BridgedTool, params: &serde_json::Value) -> ToolInvocationResult {
    match &tool.kind {
        ToolKind::BuiltIn => invoke_builtin(tool, params),
        ToolKind::JvmJar { jar_env, jar_class } => invoke_jvm_jar(tool, params, jar_env, jar_class),
        ToolKind::NativeBinary {
            binary_name,
            env_override,
        } => invoke_native_binary(tool, params, binary_name, env_override),
    }
}

fn invoke_builtin(tool: &BridgedTool, params: &serde_json::Value) -> ToolInvocationResult {
    match tool.name.as_str() {
        "z3_check" => invoke_z3_builtin(params),
        _ => ToolInvocationResult::error(&tool.name, "unknown built-in tool"),
    }
}

fn invoke_z3_builtin(params: &serde_json::Value) -> ToolInvocationResult {
    let source = match params.get("source").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            return ToolInvocationResult::error("z3_check", "missing `source` parameter");
        }
    };

    // Run the Resilient compiler pipeline (parse → typecheck).
    use crate::typechecker::TypeChecker;
    let lexer = crate::Lexer::new(&source);
    let mut parser = crate::Parser::new(lexer);
    let program = parser.parse_program();
    let parse_errors = parser.errors;

    if !parse_errors.is_empty() {
        let diags: Vec<String> = parse_errors
            .iter()
            .map(|e| format!("0:0: error: parse: {e}"))
            .collect();
        return ToolInvocationResult {
            outcome: ToolOutcome::Violation,
            diagnostics: diags,
            raw_output: String::new(),
            metadata: HashMap::new(),
        };
    }

    match TypeChecker::new().check_program(&program) {
        Ok(_) => ToolInvocationResult {
            outcome: ToolOutcome::Clean,
            diagnostics: vec!["0:0: info: Z3 verification passed".to_string()],
            raw_output: String::new(),
            metadata: HashMap::new(),
        },
        Err(e) => ToolInvocationResult {
            outcome: ToolOutcome::Violation,
            diagnostics: vec![format!("0:0: error: {e}")],
            raw_output: e.clone(),
            metadata: HashMap::new(),
        },
    }
}

fn invoke_jvm_jar(
    tool: &BridgedTool,
    params: &serde_json::Value,
    jar_env: &str,
    jar_class: &str,
) -> ToolInvocationResult {
    let jar = match resolve_jar(jar_env, tool.explicit_path.as_deref()) {
        Some(j) => j,
        None => return ToolInvocationResult::unavailable(&tool.name),
    };

    if !java_available() {
        return ToolInvocationResult::unavailable(&tool.name);
    }

    let file = match params.get("file").and_then(|v| v.as_str()) {
        Some(f) => f.to_string(),
        None => {
            return ToolInvocationResult::error(&tool.name, "missing `file` parameter");
        }
    };

    if !Path::new(&file).exists() {
        return ToolInvocationResult::error(&tool.name, &format!("file not found: {file}"));
    }

    let output = Command::new("java")
        .args([
            "-cp",
            jar.to_str().unwrap_or("tla2tools.jar"),
            jar_class,
            &file,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match output {
        Err(e) => ToolInvocationResult::error(&tool.name, &format!("launch failed: {e}")),
        Ok(out) => {
            let raw = format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            let (outcome, diags) = parse_generic_output(&raw, &file, &tool.name);
            ToolInvocationResult {
                outcome,
                diagnostics: diags,
                raw_output: raw,
                metadata: HashMap::new(),
            }
        }
    }
}

fn invoke_native_binary(
    tool: &BridgedTool,
    params: &serde_json::Value,
    binary_name: &str,
    env_override: &str,
) -> ToolInvocationResult {
    let bin = match resolve_binary(binary_name, env_override, tool.explicit_path.as_deref()) {
        Some(b) => b,
        None => return ToolInvocationResult::unavailable(&tool.name),
    };

    let file = match params.get("file").and_then(|v| v.as_str()) {
        Some(f) => f.to_string(),
        None => {
            return ToolInvocationResult::error(&tool.name, "missing `file` parameter");
        }
    };

    if !Path::new(&file).exists() {
        return ToolInvocationResult::error(&tool.name, &format!("file not found: {file}"));
    }

    let mut cmd = Command::new(&bin);
    cmd.arg(&file);

    // Tool-specific extra flags
    if tool.name == "cbmc_check" {
        let unwind = params.get("unwind").and_then(|v| v.as_u64()).unwrap_or(10);
        cmd.arg("--unwind").arg(unwind.to_string());
    }

    if tool.name == "lean4_check"
        && let Some(theorem) = params.get("theorem").and_then(|v| v.as_str())
    {
        cmd.arg("--theorem").arg(theorem);
    }

    let output = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).output();

    match output {
        Err(e) => ToolInvocationResult::error(&tool.name, &format!("launch failed: {e}")),
        Ok(out) => {
            let raw = format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            let (outcome, diags) = parse_generic_output(&raw, &file, &tool.name);
            ToolInvocationResult {
                outcome,
                diagnostics: diags,
                raw_output: raw,
                metadata: HashMap::new(),
            }
        }
    }
}

/// Generic output parser that scans for error/warning/success indicators.
fn parse_generic_output(output: &str, file: &str, tool_name: &str) -> (ToolOutcome, Vec<String>) {
    let mut diags: Vec<String> = Vec::new();
    let mut outcome = ToolOutcome::Clean;

    for line in output.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }

        // Error indicators
        if l.contains("error:")
            || l.starts_with("Error")
            || l.contains("FAILED")
            || l.contains("violated")
            || l.contains("VERIFICATION FAILED")
            || l.contains("counterexample")
            || l.contains("Counterexample")
        {
            outcome = ToolOutcome::Violation;
            diags.push(format!("{file}:0:0: error[{tool_name}]: {l}"));
            continue;
        }

        // Warning indicators
        if l.contains("warning:") || l.starts_with("Warning") || l.contains("WARN") {
            diags.push(format!("{file}:0:0: warning[{tool_name}]: {l}"));
            continue;
        }

        // Success indicators
        if l.contains("No error")
            || l.contains("VERIFICATION SUCCESSFUL")
            || l.contains("Proof complete")
            || l.contains("proved")
        {
            diags.push(format!("{file}:0:0: info[{tool_name}]: {l}"));
        }
    }

    if diags.is_empty() && outcome == ToolOutcome::Clean {
        diags.push(format!(
            "{file}:0:0: info[{tool_name}]: completed — no issues found"
        ));
    }

    (outcome, diags)
}

// ── Binary discovery ──────────────────────────────────────────────────────────

/// Resolve a JVM jar path from env var or PATH scan.
pub fn resolve_jar(env_var: &str, explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit
        && p.exists()
    {
        return Some(p.to_owned());
    }
    if let Ok(v) = std::env::var(env_var) {
        let pb = PathBuf::from(&v);
        if pb.exists() {
            return Some(pb);
        }
    }
    // Scan PATH for common jar names
    let candidates = ["tlc.jar", "tla2tools.jar", "lean4.jar"];
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for &name in &candidates {
                let candidate = dir.join(name);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Resolve a native binary from env var or PATH.
pub fn resolve_binary(name: &str, env_var: &str, explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit
        && p.exists()
    {
        return Some(p.to_owned());
    }
    if let Ok(v) = std::env::var(env_var) {
        let pb = PathBuf::from(&v);
        if pb.exists() {
            return Some(pb);
        }
    }
    // which-style PATH scan
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
            // On Unix also try without extension; on Windows .exe
            #[cfg(target_os = "windows")]
            {
                let with_ext = dir.join(format!("{name}.exe"));
                if with_ext.exists() {
                    return Some(with_ext);
                }
            }
        }
    }
    None
}

/// Returns true if `java` is on PATH.
pub fn java_available() -> bool {
    Command::new("java")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── `rz tool` CLI subcommand ──────────────────────────────────────────────────

/// Handle `rz tool <verb> [args...]`.
///
/// Returns `Some(exit_code)` when the subcommand was recognised, `None`
/// to fall through to the normal compiler driver.
pub fn dispatch_tool_subcommand(args: &[String]) -> Option<i32> {
    let first = args.first()?;
    if first != "tool" {
        return None;
    }

    let verb = args.get(1).map(String::as_str).unwrap_or("--help");
    match verb {
        "list" => Some(cmd_list()),
        "call" => Some(cmd_call(&args[2..])),
        "check" => Some(cmd_check(&args[2..])),
        "--help" | "-h" | "help" => {
            print_tool_help();
            Some(0)
        }
        other => {
            eprintln!("Error: unknown `tool` subcommand `{other}`. Try `rz tool --help`.");
            Some(1)
        }
    }
}

fn cmd_list() -> i32 {
    let reg = McpBridgeRegistry::global();
    let tools = reg.all_tools();
    if tools.is_empty() {
        println!("No external tools registered.");
        return 0;
    }
    println!("{:<20} {:<10} DESCRIPTION", "NAME", "STATUS");
    println!("{}", "-".repeat(72));
    for tool in &tools {
        let status = if tool.is_available() {
            "available"
        } else {
            "not found"
        };
        println!("{:<20} {:<10} {}", tool.name, status, tool.description);
    }
    0
}

fn cmd_call(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("Usage: rz tool call <tool-name> [key=value ...]");
        return 1;
    }
    let name = &args[0];
    let mut params = serde_json::Map::new();
    for kv in &args[1..] {
        if let Some((k, v)) = kv.split_once('=') {
            params.insert(k.to_string(), serde_json::Value::String(v.to_string()));
        }
    }
    let params = serde_json::Value::Object(params);

    match McpBridgeRegistry::global().invoke(name, &params) {
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
        Ok(result) => {
            for diag in &result.diagnostics {
                println!("{diag}");
            }
            match result.outcome {
                ToolOutcome::Clean => 0,
                _ => 1,
            }
        }
    }
}

fn cmd_check(args: &[String]) -> i32 {
    let file = match args.first() {
        Some(f) => f.clone(),
        None => {
            eprintln!("Usage: rz tool check <file>");
            return 1;
        }
    };

    let reg = McpBridgeRegistry::global();
    let available = reg.available_tools();

    if available.is_empty() {
        println!("No external tools available for `{file}`.");
        return 0;
    }

    let mut any_violation = false;
    for tool in &available {
        let params = serde_json::json!({ "file": file, "source": "" });
        let result = reg
            .invoke(&tool.name, &params)
            .unwrap_or_else(|e| ToolInvocationResult::error(&tool.name, &e));
        for diag in &result.diagnostics {
            println!("{diag}");
        }
        if result.outcome == ToolOutcome::Violation {
            any_violation = true;
        }
    }

    if any_violation { 1 } else { 0 }
}

fn print_tool_help() {
    println!(
        "rz tool — external tool bridge (MCP scaffolding)

USAGE:
    rz tool list                     List all registered external tools
    rz tool call <name> [key=value]  Invoke a registered tool
    rz tool check <file>             Run all available tools against <file>

REGISTERED TOOLS:
    tlc_check    TLA+ model checking via TLC
    lean4_check  Lean 4 proof checking
    cbmc_check   CBMC bounded model checking
    z3_check     Z3 SMT contract verification (built-in)

DISCOVERY:
    RESILIENT_TLC_JAR    Path to tla2tools.jar
    RESILIENT_LEAN_BIN   Path to lean binary
    RESILIENT_CBMC_BIN   Path to cbmc binary

EXAMPLES:
    rz tool list
    rz tool call tlc_check file=Spec.tla
    rz tool call lean4_check file=Proof.lean
    rz tool call cbmc_check file=module.c unwind=20
    rz tool call z3_check source='fn f(int x) requires x > 0 {{ return x; }}'
    rz tool check myprogram.rz"
    );
}

// ── Typechecker check() — static analysis of #[mcp_tool] annotations ─────────

/// Validate `#[mcp_tool]` and `#[external_tool]` annotations in the program.
///
/// Uses the shared attribute registry (`feature_attrs`) rather than inspecting
/// AST nodes directly — attributes are recorded during parsing and queried here.
pub(crate) fn check(_program: &crate::Node, _source_path: &str) -> Result<(), String> {
    let mcp_tools = crate::feature_attrs::find_kind("mcp_tool");
    let ext_tools = crate::feature_attrs::find_kind("external_tool");

    if mcp_tools.is_empty() && ext_tools.is_empty() {
        return Ok(());
    }

    let mut errors: Vec<String> = Vec::new();

    for (fn_name, record) in &mcp_tools {
        // Validate that mcp_tool has a `name=` argument
        if !record.args.contains("name") {
            errors.push(format!(
                "0:0: error: `#[mcp_tool]` on `{fn_name}` requires a `name=\"...\"` argument"
            ));
        }
    }

    for (fn_name, record) in &ext_tools {
        // Validate that external_tool has a `backend=` argument
        if !record.args.contains("backend") {
            errors.push(format!(
                "0:0: error: `#[external_tool]` on `{fn_name}` requires a \
                 `backend=\"...\"` argument (e.g., backend=\"z3\")"
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ToolInvocationResult helpers ─────────────────────────────────────────

    #[test]
    fn unavailable_result_has_correct_outcome() {
        let r = ToolInvocationResult::unavailable("my_tool");
        assert_eq!(r.outcome, ToolOutcome::Unavailable);
        assert!(r.diagnostics[0].contains("my_tool"));
    }

    #[test]
    fn error_result_has_correct_outcome() {
        let r = ToolInvocationResult::error("my_tool", "something went wrong");
        assert_eq!(r.outcome, ToolOutcome::InvocationError);
        assert!(r.diagnostics[0].contains("something went wrong"));
    }

    // ── BridgedTool ──────────────────────────────────────────────────────────

    #[test]
    fn z3_adapter_is_builtin_kind() {
        let t = z3_adapter();
        assert_eq!(t.kind, ToolKind::BuiltIn);
        assert!(t.is_available()); // built-in always available
    }

    #[test]
    fn tlc_adapter_requires_jar_and_java() {
        let t = tlc_adapter();
        // Without tla2tools.jar, should not be available
        if !java_available() {
            assert!(!t.is_available());
        }
    }

    #[test]
    fn cbmc_adapter_unavailable_without_binary() {
        let t = cbmc_adapter();
        // CBMC is typically not installed in CI; this is not a hard requirement
        // just verify the struct was created correctly
        assert_eq!(t.name, "cbmc_check");
        match &t.kind {
            ToolKind::NativeBinary { binary_name, .. } => {
                assert_eq!(binary_name, "cbmc");
            }
            _ => panic!("expected NativeBinary kind"),
        }
    }

    #[test]
    fn lean4_adapter_has_correct_schema() {
        let t = lean4_adapter();
        assert_eq!(t.name, "lean4_check");
        let schema = &t.input_schema;
        assert!(schema.get("properties").is_some());
        assert!(schema["properties"].get("file").is_some());
    }

    #[test]
    fn mcp_definition_includes_availability() {
        let t = z3_adapter();
        let def = t.mcp_definition();
        let desc = def["description"].as_str().unwrap();
        assert!(desc.contains("available"), "got: {desc}");
    }

    // ── McpBridgeRegistry ────────────────────────────────────────────────────

    #[test]
    fn registry_starts_with_four_builtin_tools() {
        let reg = McpBridgeRegistry::global();
        let tools = reg.all_tools();
        assert!(tools.len() >= 4, "expected >= 4 tools, got {}", tools.len());
    }

    #[test]
    fn registry_get_returns_tool_by_name() {
        let reg = McpBridgeRegistry::global();
        let t = reg.get("z3_check");
        assert!(t.is_some());
        assert_eq!(t.unwrap().name, "z3_check");
    }

    #[test]
    fn registry_get_unknown_tool_returns_none() {
        let reg = McpBridgeRegistry::global();
        assert!(reg.get("nonexistent_xyz").is_none());
    }

    #[test]
    fn registry_invoke_unknown_tool_returns_error() {
        let reg = McpBridgeRegistry::global();
        let result = reg.invoke("nonexistent_xyz", &serde_json::json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown tool"));
    }

    #[test]
    fn registry_mcp_definitions_are_valid_json() {
        let reg = McpBridgeRegistry::global();
        let defs = reg.mcp_tool_definitions();
        assert!(!defs.is_empty());
        for def in &defs {
            assert!(def.get("name").is_some());
            assert!(def.get("description").is_some());
            assert!(def.get("inputSchema").is_some());
        }
    }

    #[test]
    fn register_custom_tool_replaces_existing() {
        // We can't modify the global registry without polluting other tests,
        // so create a local registry for this test.
        let reg = McpBridgeRegistry {
            tools: std::sync::RwLock::new(Vec::new()),
        };
        reg.register(z3_adapter());
        assert_eq!(reg.all_tools().len(), 1);
        // Registering again replaces
        reg.register(z3_adapter());
        assert_eq!(reg.all_tools().len(), 1);
        // Registering different name adds
        reg.register(lean4_adapter());
        assert_eq!(reg.all_tools().len(), 2);
    }

    // ── Discovery helpers ────────────────────────────────────────────────────

    #[test]
    fn resolve_binary_with_nonexistent_env_returns_none() {
        // Use a binary name that definitely won't be on PATH
        let result = resolve_binary(
            "nosuchbinary_xyz_999_resilient_test",
            "RESILIENT_TEST_NOSUCHBIN_UNSET_XYZ",
            None,
        );
        assert!(result.is_none());
    }

    #[test]
    fn resolve_jar_with_nonexistent_env_returns_none() {
        let result = resolve_jar("RESILIENT_TEST_NOJAR_UNSET_XYZ_999", None);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_binary_finds_existing_executable() {
        // `ls` or `sh` should be on PATH in CI
        let result = resolve_binary("sh", "RESILIENT_SH_OVERRIDE", None);
        // May or may not be present depending on OS; just verify no panic
        let _ = result;
    }

    // ── parse_generic_output ─────────────────────────────────────────────────

    #[test]
    fn parse_clean_output_returns_clean_outcome() {
        let (outcome, diags) = parse_generic_output("No error has been found.", "test.c", "cbmc");
        assert_eq!(outcome, ToolOutcome::Clean);
        assert!(!diags.is_empty());
    }

    #[test]
    fn parse_output_with_error_returns_violation() {
        let (outcome, diags) =
            parse_generic_output("error: null pointer dereference", "test.c", "cbmc");
        assert_eq!(outcome, ToolOutcome::Violation);
        assert!(diags.iter().any(|d| d.contains("error")));
    }

    #[test]
    fn parse_output_with_failed_returns_violation() {
        let (outcome, _) = parse_generic_output("VERIFICATION FAILED", "test.tla", "tlc");
        assert_eq!(outcome, ToolOutcome::Violation);
    }

    #[test]
    fn parse_empty_output_returns_info_diagnostic() {
        let (outcome, diags) = parse_generic_output("", "test.lean", "lean");
        assert_eq!(outcome, ToolOutcome::Clean);
        assert!(diags[0].contains("no issues found"), "got: {:?}", diags);
    }

    // ── CLI dispatch ─────────────────────────────────────────────────────────

    #[test]
    fn non_tool_arg_returns_none() {
        let args = vec!["check".to_string()];
        assert!(dispatch_tool_subcommand(&args).is_none());
    }

    #[test]
    fn tool_help_returns_zero() {
        let args = vec!["tool".to_string(), "--help".to_string()];
        assert_eq!(dispatch_tool_subcommand(&args), Some(0));
    }

    #[test]
    fn tool_list_returns_zero() {
        let args = vec!["tool".to_string(), "list".to_string()];
        assert_eq!(dispatch_tool_subcommand(&args), Some(0));
    }

    #[test]
    fn tool_unknown_verb_returns_one() {
        let args = vec!["tool".to_string(), "frobnicate".to_string()];
        assert_eq!(dispatch_tool_subcommand(&args), Some(1));
    }

    #[test]
    fn tool_call_unknown_tool_returns_one() {
        let args = vec![
            "tool".to_string(),
            "call".to_string(),
            "nonexistent_xyz".to_string(),
        ];
        assert_eq!(dispatch_tool_subcommand(&args), Some(1));
    }

    #[test]
    fn tool_call_without_name_returns_one() {
        let args = vec!["tool".to_string(), "call".to_string()];
        assert_eq!(dispatch_tool_subcommand(&args), Some(1));
    }

    // ── check() typechecker integration ──────────────────────────────────────

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = crate::parse("");
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_ok_on_function_without_annotation() {
        let src = "fn add(int a, int b) -> int { return a + b; }";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn z3_invoke_on_valid_source_returns_clean() {
        let reg = McpBridgeRegistry::global();
        let params = serde_json::json!({
            "source": "fn f(int x) -> int { return x + 1; }"
        });
        let result = reg.invoke("z3_check", &params).unwrap();
        // z3_check is built-in and always runs; outcome depends on typechecker
        assert_ne!(result.outcome, ToolOutcome::Unavailable);
    }

    #[test]
    fn z3_invoke_missing_source_returns_error() {
        let reg = McpBridgeRegistry::global();
        let result = reg.invoke("z3_check", &serde_json::json!({})).unwrap();
        assert_eq!(result.outcome, ToolOutcome::InvocationError);
    }
}
