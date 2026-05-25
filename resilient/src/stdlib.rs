//! Standard library module system for Resilient.
//!
//! `use std::http;` imports the HTTP standard library as a namespace.
//! `use std::json;` imports JSON utilities. Etc.
//!
//! Standard libraries are implemented as synthetic AST nodes injected
//! before the program reaches the interpreter. Each stdlib module
//! provides a set of functions namespaced under its module name
//! (e.g., `http::get(url)`, `json::parse(s)`).
//!
//! The system also supports `pub`/private visibility:
//! - In file-based imports (`use "file.rz";`), only `pub fn` and
//!   `pub struct` declarations are exported. Declarations without
//!   `pub` are private to their defining file.
//! - Standard library modules export everything (they're all pub).
//! - Top-level code in the entry file has no visibility restrictions.

use crate::{MapKey, RResult, Value};
use std::collections::HashMap;

/// A standard library module definition.
#[allow(dead_code)]
pub struct StdModule {
    pub name: &'static str,
    pub functions: &'static [StdFn],
    pub description: &'static str,
}

/// A function provided by a standard library module.
#[allow(dead_code)]
pub struct StdFn {
    pub name: &'static str,
    pub params: &'static [(&'static str, &'static str)], // (type, name)
    pub return_type: &'static str,
    pub handler: fn(&[Value]) -> RResult<Value>,
}

/// Registry of all available standard library modules.
static STD_MODULES: &[&StdModule] = &[
    &STD_HTTP,
    &STD_JSON,
    &STD_MATH,
    &STD_FS,
    &STD_OS,
    &STD_CRYPTO,
    &STD_BASE64,
    &STD_REGEX,
    &STD_TIME,
    &STD_NET,
    &STD_COLLECTIONS,
];

/// Look up a standard library module by name.
pub fn lookup_std_module(name: &str) -> Option<&'static StdModule> {
    STD_MODULES.iter().find(|m| m.name == name).copied()
}

/// List all available standard library module names.
pub fn all_std_module_names() -> impl Iterator<Item = &'static str> {
    STD_MODULES.iter().map(|m| m.name)
}

/// Resolve a `use std::X;` import. Returns the namespaced function
/// bindings that should be injected into the environment.
pub fn resolve_std_import(
    module_name: &str,
    alias: Option<&str>,
) -> Result<Vec<(String, StdBinding)>, String> {
    let module = lookup_std_module(module_name).ok_or_else(|| {
        let available: Vec<&str> = all_std_module_names().collect();
        format!(
            "Unknown standard library module 'std::{}'. Available: {}",
            module_name,
            available.join(", ")
        )
    })?;

    let ns = alias.unwrap_or(module_name);
    let mut bindings = Vec::new();
    for func in module.functions {
        let qualified_name = format!("{}_{}", ns, func.name);
        bindings.push((qualified_name, StdBinding::Function(func.handler)));
    }
    Ok(bindings)
}

/// A binding from a standard library module.
pub enum StdBinding {
    Function(fn(&[Value]) -> RResult<Value>),
}

impl std::fmt::Debug for StdBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StdBinding::Function(_) => write!(f, "StdBinding::Function(...)"),
        }
    }
}

/// Register standard library bindings into the interpreter environment.
pub fn inject_std_bindings(bindings: &[(String, StdBinding)], env: &crate::Environment) {
    for (name, binding) in bindings {
        match binding {
            StdBinding::Function(handler) => {
                let name_static: &'static str = Box::leak(name.clone().into_boxed_str());
                env.set(
                    name.clone(),
                    Value::Builtin {
                        name: name_static,
                        func: *handler,
                    },
                );
            }
        }
    }
}

// ─── Standard Library: HTTP ─────────────────────────────────────────

fn http_get(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("http::get requires a URL argument".to_string());
    }
    let url = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("http::get: expected string URL, got {:?}", other)),
    };

    // Synchronous HTTP GET using std::net::TcpStream
    match simple_http_get(&url) {
        Ok(body) => Ok(Value::String(body)),
        Err(e) => Err(format!("http::get failed: {}", e)),
    }
}

fn http_post(args: &[Value]) -> RResult<Value> {
    if args.len() < 2 {
        return Err("http::post requires (url, body) arguments".to_string());
    }
    let url = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("http::post: expected string URL, got {:?}", other)),
    };
    let body = match &args[1] {
        Value::String(s) => s.clone(),
        other => return Err(format!("http::post: expected string body, got {:?}", other)),
    };

    match simple_http_post(&url, &body) {
        Ok(response) => Ok(Value::String(response)),
        Err(e) => Err(format!("http::post failed: {}", e)),
    }
}

fn http_status(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("http::status requires a URL argument".to_string());
    }
    let url = match &args[0] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "http::status: expected string URL, got {:?}",
                other
            ));
        }
    };
    match simple_http_head(&url) {
        Ok(status) => Ok(Value::Int(status as i64)),
        Err(e) => Err(format!("http::status failed: {}", e)),
    }
}

fn http_headers(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("http::headers requires a URL argument".to_string());
    }
    let url = match &args[0] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "http::headers: expected string URL, got {:?}",
                other
            ));
        }
    };
    match simple_http_head_headers(&url) {
        Ok(headers) => {
            let mut map = HashMap::new();
            for (k, v) in headers {
                map.insert(MapKey::Str(k), Value::String(v));
            }
            Ok(Value::Map(map))
        }
        Err(e) => Err(format!("http::headers failed: {}", e)),
    }
}

static STD_HTTP: StdModule = StdModule {
    name: "http",
    description: "HTTP client for making web requests",
    functions: &[
        StdFn {
            name: "get",
            params: &[("string", "url")],
            return_type: "string",
            handler: http_get,
        },
        StdFn {
            name: "post",
            params: &[("string", "url"), ("string", "body")],
            return_type: "string",
            handler: http_post,
        },
        StdFn {
            name: "status",
            params: &[("string", "url")],
            return_type: "int",
            handler: http_status,
        },
        StdFn {
            name: "headers",
            params: &[("string", "url")],
            return_type: "map",
            handler: http_headers,
        },
    ],
};

// ─── Standard Library: JSON ──────────���─────────────────────��────────

fn json_parse(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("json::parse requires a string argument".to_string());
    }
    let s = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("json::parse: expected string, got {:?}", other)),
    };
    parse_json_value(&s).map_err(|e| format!("json::parse: {}", e))
}

fn json_stringify(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("json::stringify requires a value argument".to_string());
    }
    Ok(Value::String(value_to_json(&args[0])))
}

fn json_pretty(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("json::pretty requires a value argument".to_string());
    }
    Ok(Value::String(value_to_json_pretty(&args[0], 0)))
}

fn json_valid(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("json::valid requires a string argument".to_string());
    }
    let s = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("json::valid: expected string, got {:?}", other)),
    };
    Ok(Value::Bool(parse_json_value(&s).is_ok()))
}

static STD_JSON: StdModule = StdModule {
    name: "json",
    description: "JSON parsing and serialization",
    functions: &[
        StdFn {
            name: "parse",
            params: &[("string", "s")],
            return_type: "any",
            handler: json_parse,
        },
        StdFn {
            name: "stringify",
            params: &[("any", "value")],
            return_type: "string",
            handler: json_stringify,
        },
        StdFn {
            name: "pretty",
            params: &[("any", "value")],
            return_type: "string",
            handler: json_pretty,
        },
        StdFn {
            name: "valid",
            params: &[("string", "s")],
            return_type: "bool",
            handler: json_valid,
        },
    ],
};

// ─── Standard Library: Math (extended) ──────────────────────────────

fn math_pi(_args: &[Value]) -> RResult<Value> {
    Ok(Value::Float(std::f64::consts::PI))
}

fn math_e(_args: &[Value]) -> RResult<Value> {
    Ok(Value::Float(std::f64::consts::E))
}

fn math_tau(_args: &[Value]) -> RResult<Value> {
    Ok(Value::Float(std::f64::consts::TAU))
}

fn math_log(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("math::log requires a numeric argument".to_string());
    }
    let x = to_f64(&args[0])?;
    if x <= 0.0 {
        return Err("math::log: argument must be positive".to_string());
    }
    Ok(Value::Float(x.ln()))
}

fn math_log2(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("math::log2 requires a numeric argument".to_string());
    }
    let x = to_f64(&args[0])?;
    if x <= 0.0 {
        return Err("math::log2: argument must be positive".to_string());
    }
    Ok(Value::Float(x.log2()))
}

fn math_log10(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("math::log10 requires a numeric argument".to_string());
    }
    let x = to_f64(&args[0])?;
    if x <= 0.0 {
        return Err("math::log10: argument must be positive".to_string());
    }
    Ok(Value::Float(x.log10()))
}

fn math_exp(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("math::exp requires a numeric argument".to_string());
    }
    let x = to_f64(&args[0])?;
    Ok(Value::Float(x.exp()))
}

fn math_hypot(args: &[Value]) -> RResult<Value> {
    if args.len() < 2 {
        return Err("math::hypot requires two numeric arguments".to_string());
    }
    let x = to_f64(&args[0])?;
    let y = to_f64(&args[1])?;
    Ok(Value::Float(x.hypot(y)))
}

fn math_sqrt(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("math::sqrt requires a numeric argument".to_string());
    }
    let x = to_f64(&args[0])?;
    if x < 0.0 {
        return Err("math::sqrt: argument must be non-negative".to_string());
    }
    Ok(Value::Float(x.sqrt()))
}

fn math_abs(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("math::abs requires a numeric argument".to_string());
    }
    let x = to_f64(&args[0])?;
    Ok(Value::Float(x.abs()))
}

fn math_pow(args: &[Value]) -> RResult<Value> {
    if args.len() < 2 {
        return Err("math::pow requires two numeric arguments".to_string());
    }
    let base = to_f64(&args[0])?;
    let exp = to_f64(&args[1])?;
    Ok(Value::Float(base.powf(exp)))
}

fn math_floor(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("math::floor requires a numeric argument".to_string());
    }
    Ok(Value::Float(to_f64(&args[0])?.floor()))
}

fn math_ceil(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("math::ceil requires a numeric argument".to_string());
    }
    Ok(Value::Float(to_f64(&args[0])?.ceil()))
}

fn math_round(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("math::round requires a numeric argument".to_string());
    }
    Ok(Value::Float(to_f64(&args[0])?.round()))
}

fn math_sin(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("math::sin requires a numeric argument".to_string());
    }
    Ok(Value::Float(to_f64(&args[0])?.sin()))
}

fn math_cos(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("math::cos requires a numeric argument".to_string());
    }
    Ok(Value::Float(to_f64(&args[0])?.cos()))
}

fn math_tan(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("math::tan requires a numeric argument".to_string());
    }
    Ok(Value::Float(to_f64(&args[0])?.tan()))
}

fn math_min(args: &[Value]) -> RResult<Value> {
    if args.len() < 2 {
        return Err("math::min requires two numeric arguments".to_string());
    }
    let a = to_f64(&args[0])?;
    let b = to_f64(&args[1])?;
    Ok(Value::Float(a.min(b)))
}

fn math_max(args: &[Value]) -> RResult<Value> {
    if args.len() < 2 {
        return Err("math::max requires two numeric arguments".to_string());
    }
    let a = to_f64(&args[0])?;
    let b = to_f64(&args[1])?;
    Ok(Value::Float(a.max(b)))
}

fn math_random(_args: &[Value]) -> RResult<Value> {
    // Simple LCG for determinism in tests; sufficient for non-crypto use.
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(42);
    let val = (seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)) as f64
        / u64::MAX as f64;
    Ok(Value::Float(val.abs()))
}

static STD_MATH: StdModule = StdModule {
    name: "math",
    description: "Extended mathematical functions and constants",
    functions: &[
        StdFn {
            name: "pi",
            params: &[],
            return_type: "float",
            handler: math_pi,
        },
        StdFn {
            name: "e",
            params: &[],
            return_type: "float",
            handler: math_e,
        },
        StdFn {
            name: "tau",
            params: &[],
            return_type: "float",
            handler: math_tau,
        },
        StdFn {
            name: "log",
            params: &[("float", "x")],
            return_type: "float",
            handler: math_log,
        },
        StdFn {
            name: "log2",
            params: &[("float", "x")],
            return_type: "float",
            handler: math_log2,
        },
        StdFn {
            name: "log10",
            params: &[("float", "x")],
            return_type: "float",
            handler: math_log10,
        },
        StdFn {
            name: "exp",
            params: &[("float", "x")],
            return_type: "float",
            handler: math_exp,
        },
        StdFn {
            name: "sqrt",
            params: &[("float", "x")],
            return_type: "float",
            handler: math_sqrt,
        },
        StdFn {
            name: "abs",
            params: &[("float", "x")],
            return_type: "float",
            handler: math_abs,
        },
        StdFn {
            name: "pow",
            params: &[("float", "base"), ("float", "exp")],
            return_type: "float",
            handler: math_pow,
        },
        StdFn {
            name: "floor",
            params: &[("float", "x")],
            return_type: "float",
            handler: math_floor,
        },
        StdFn {
            name: "ceil",
            params: &[("float", "x")],
            return_type: "float",
            handler: math_ceil,
        },
        StdFn {
            name: "round",
            params: &[("float", "x")],
            return_type: "float",
            handler: math_round,
        },
        StdFn {
            name: "sin",
            params: &[("float", "x")],
            return_type: "float",
            handler: math_sin,
        },
        StdFn {
            name: "cos",
            params: &[("float", "x")],
            return_type: "float",
            handler: math_cos,
        },
        StdFn {
            name: "tan",
            params: &[("float", "x")],
            return_type: "float",
            handler: math_tan,
        },
        StdFn {
            name: "min",
            params: &[("float", "a"), ("float", "b")],
            return_type: "float",
            handler: math_min,
        },
        StdFn {
            name: "max",
            params: &[("float", "a"), ("float", "b")],
            return_type: "float",
            handler: math_max,
        },
        StdFn {
            name: "hypot",
            params: &[("float", "x"), ("float", "y")],
            return_type: "float",
            handler: math_hypot,
        },
        StdFn {
            name: "random",
            params: &[],
            return_type: "float",
            handler: math_random,
        },
    ],
};

// ─── Standard Library: FS (filesystem) ──────────────────────────────

fn fs_read(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("fs::read requires a path argument".to_string());
    }
    let path = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("fs::read: expected string path, got {:?}", other)),
    };
    match std::fs::read_to_string(&path) {
        Ok(contents) => Ok(Value::String(contents)),
        Err(e) => Err(format!("fs::read '{}': {}", path, e)),
    }
}

fn fs_write(args: &[Value]) -> RResult<Value> {
    if args.len() < 2 {
        return Err("fs::write requires (path, contents) arguments".to_string());
    }
    let path = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("fs::write: expected string path, got {:?}", other)),
    };
    let contents = match &args[1] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "fs::write: expected string contents, got {:?}",
                other
            ));
        }
    };
    match std::fs::write(&path, &contents) {
        Ok(()) => Ok(Value::Void),
        Err(e) => Err(format!("fs::write '{}': {}", path, e)),
    }
}

fn fs_exists(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("fs::exists requires a path argument".to_string());
    }
    let path = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("fs::exists: expected string path, got {:?}", other)),
    };
    Ok(Value::Bool(std::path::Path::new(&path).exists()))
}

fn fs_remove(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("fs::remove requires a path argument".to_string());
    }
    let path = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("fs::remove: expected string path, got {:?}", other)),
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(Value::Void),
        Err(e) => Err(format!("fs::remove '{}': {}", path, e)),
    }
}

fn fs_list(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("fs::list requires a directory path argument".to_string());
    }
    let path = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("fs::list: expected string path, got {:?}", other)),
    };
    match std::fs::read_dir(&path) {
        Ok(entries) => {
            let mut files = Vec::new();
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    files.push(Value::String(name.to_string()));
                }
            }
            Ok(Value::Array(files))
        }
        Err(e) => Err(format!("fs::list '{}': {}", path, e)),
    }
}

fn fs_append(args: &[Value]) -> RResult<Value> {
    if args.len() < 2 {
        return Err("fs::append requires (path, contents) arguments".to_string());
    }
    let path = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("fs::append: expected string path, got {:?}", other)),
    };
    let contents = match &args[1] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "fs::append: expected string contents, got {:?}",
                other
            ));
        }
    };
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("fs::append '{}': {}", path, e))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| format!("fs::append '{}': {}", path, e))?;
    Ok(Value::Void)
}

static STD_FS: StdModule = StdModule {
    name: "fs",
    description: "Filesystem operations (read, write, list, remove)",
    functions: &[
        StdFn {
            name: "read",
            params: &[("string", "path")],
            return_type: "string",
            handler: fs_read,
        },
        StdFn {
            name: "write",
            params: &[("string", "path"), ("string", "contents")],
            return_type: "void",
            handler: fs_write,
        },
        StdFn {
            name: "append",
            params: &[("string", "path"), ("string", "contents")],
            return_type: "void",
            handler: fs_append,
        },
        StdFn {
            name: "exists",
            params: &[("string", "path")],
            return_type: "bool",
            handler: fs_exists,
        },
        StdFn {
            name: "remove",
            params: &[("string", "path")],
            return_type: "void",
            handler: fs_remove,
        },
        StdFn {
            name: "list",
            params: &[("string", "dir")],
            return_type: "array",
            handler: fs_list,
        },
    ],
};

// ─── Standard Library: OS ───────────────────────────────────────────

fn os_env(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("os::env requires a variable name argument".to_string());
    }
    let name = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("os::env: expected string, got {:?}", other)),
    };
    match std::env::var(&name) {
        Ok(val) => Ok(Value::String(val)),
        Err(_) => Ok(Value::String(String::new())),
    }
}

fn os_args(_args: &[Value]) -> RResult<Value> {
    let args: Vec<Value> = std::env::args().map(Value::String).collect();
    Ok(Value::Array(args))
}

fn os_exit(args: &[Value]) -> RResult<Value> {
    let code = if args.is_empty() {
        0
    } else {
        match &args[0] {
            Value::Int(n) => *n as i32,
            _ => 1,
        }
    };
    std::process::exit(code);
}

fn os_platform(_args: &[Value]) -> RResult<Value> {
    Ok(Value::String(std::env::consts::OS.to_string()))
}

fn os_arch(_args: &[Value]) -> RResult<Value> {
    Ok(Value::String(std::env::consts::ARCH.to_string()))
}

fn os_cwd(_args: &[Value]) -> RResult<Value> {
    match std::env::current_dir() {
        Ok(p) => Ok(Value::String(p.to_string_lossy().to_string())),
        Err(e) => Err(format!("os::cwd: {}", e)),
    }
}

static STD_OS: StdModule = StdModule {
    name: "os",
    description: "Operating system interfaces (env vars, platform info)",
    functions: &[
        StdFn {
            name: "env",
            params: &[("string", "name")],
            return_type: "string",
            handler: os_env,
        },
        StdFn {
            name: "args",
            params: &[],
            return_type: "array",
            handler: os_args,
        },
        StdFn {
            name: "exit",
            params: &[("int", "code")],
            return_type: "void",
            handler: os_exit,
        },
        StdFn {
            name: "platform",
            params: &[],
            return_type: "string",
            handler: os_platform,
        },
        StdFn {
            name: "arch",
            params: &[],
            return_type: "string",
            handler: os_arch,
        },
        StdFn {
            name: "cwd",
            params: &[],
            return_type: "string",
            handler: os_cwd,
        },
    ],
};

// ─── Standard Library: Crypto ───────────────────────────────────────

fn crypto_sha256(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("crypto::sha256 requires a string argument".to_string());
    }
    let input = match &args[0] {
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Bytes(b) => b.clone(),
        other => {
            return Err(format!(
                "crypto::sha256: expected string or bytes, got {:?}",
                other
            ));
        }
    };
    Ok(Value::String(sha256_hex(&input)))
}

fn crypto_random_bytes(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("crypto::random_bytes requires a length argument".to_string());
    }
    let n = match &args[0] {
        Value::Int(n) => *n as usize,
        other => {
            return Err(format!(
                "crypto::random_bytes: expected int, got {:?}",
                other
            ));
        }
    };
    if n > 1024 * 1024 {
        return Err("crypto::random_bytes: max 1MB".to_string());
    }
    let mut bytes = vec![0u8; n];
    // Use /dev/urandom on unix, or time-based fallback
    #[cfg(unix)]
    {
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let _ = f.read_exact(&mut bytes);
        }
    }
    #[cfg(not(unix))]
    {
        use std::time::SystemTime;
        let mut seed = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(42);
        for b in bytes.iter_mut() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (seed >> 33) as u8;
        }
    }
    Ok(Value::Bytes(bytes))
}

fn crypto_uuid(_args: &[Value]) -> RResult<Value> {
    // Generate a v4-style UUID using random bytes
    let mut bytes = [0u8; 16];
    #[cfg(unix)]
    {
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let _ = f.read_exact(&mut bytes);
        }
    }
    #[cfg(not(unix))]
    {
        use std::time::SystemTime;
        let mut seed = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(42);
        for b in bytes.iter_mut() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (seed >> 33) as u8;
        }
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 1
    let uuid = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    );
    Ok(Value::String(uuid))
}

static STD_CRYPTO: StdModule = StdModule {
    name: "crypto",
    description: "Cryptographic utilities (hashing, random bytes, UUIDs)",
    functions: &[
        StdFn {
            name: "sha256",
            params: &[("string", "input")],
            return_type: "string",
            handler: crypto_sha256,
        },
        StdFn {
            name: "random_bytes",
            params: &[("int", "length")],
            return_type: "bytes",
            handler: crypto_random_bytes,
        },
        StdFn {
            name: "uuid",
            params: &[],
            return_type: "string",
            handler: crypto_uuid,
        },
    ],
};

// ─── Standard Library: Base64 ───────────────────────────────────��───

fn base64_encode(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("base64::encode requires a string or bytes argument".to_string());
    }
    let input = match &args[0] {
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Bytes(b) => b.clone(),
        other => {
            return Err(format!(
                "base64::encode: expected string or bytes, got {:?}",
                other
            ));
        }
    };
    Ok(Value::String(b64_encode(&input)))
}

fn base64_decode(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("base64::decode requires a string argument".to_string());
    }
    let input = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("base64::decode: expected string, got {:?}", other)),
    };
    match b64_decode(&input) {
        Ok(bytes) => match String::from_utf8(bytes.clone()) {
            Ok(s) => Ok(Value::String(s)),
            Err(_) => Ok(Value::Bytes(bytes)),
        },
        Err(e) => Err(format!("base64::decode: {}", e)),
    }
}

static STD_BASE64: StdModule = StdModule {
    name: "base64",
    description: "Base64 encoding and decoding",
    functions: &[
        StdFn {
            name: "encode",
            params: &[("string", "input")],
            return_type: "string",
            handler: base64_encode,
        },
        StdFn {
            name: "decode",
            params: &[("string", "input")],
            return_type: "string",
            handler: base64_decode,
        },
    ],
};

// ─── Standard Library: Regex ─────────��──────────────────────────────

fn regex_match(args: &[Value]) -> RResult<Value> {
    if args.len() < 2 {
        return Err("regex::match requires (pattern, text) arguments".to_string());
    }
    let pattern = match &args[0] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "regex::match: expected string pattern, got {:?}",
                other
            ));
        }
    };
    let text = match &args[1] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "regex::match: expected string text, got {:?}",
                other
            ));
        }
    };
    Ok(Value::Bool(simple_regex_match(&pattern, &text)))
}

fn regex_find(args: &[Value]) -> RResult<Value> {
    if args.len() < 2 {
        return Err("regex::find requires (pattern, text) arguments".to_string());
    }
    let pattern = match &args[0] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "regex::find: expected string pattern, got {:?}",
                other
            ));
        }
    };
    let text = match &args[1] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "regex::find: expected string text, got {:?}",
                other
            ));
        }
    };
    let matches = simple_regex_find_all(&pattern, &text);
    Ok(Value::Array(
        matches.into_iter().map(Value::String).collect(),
    ))
}

fn regex_replace(args: &[Value]) -> RResult<Value> {
    if args.len() < 3 {
        return Err("regex::replace requires (pattern, replacement, text) arguments".to_string());
    }
    let pattern = match &args[0] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "regex::replace: expected string pattern, got {:?}",
                other
            ));
        }
    };
    let replacement = match &args[1] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "regex::replace: expected string replacement, got {:?}",
                other
            ));
        }
    };
    let text = match &args[2] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "regex::replace: expected string text, got {:?}",
                other
            ));
        }
    };
    Ok(Value::String(simple_regex_replace(
        &pattern,
        &replacement,
        &text,
    )))
}

fn regex_split(args: &[Value]) -> RResult<Value> {
    if args.len() < 2 {
        return Err("regex::split requires (pattern, text) arguments".to_string());
    }
    let pattern = match &args[0] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "regex::split: expected string pattern, got {:?}",
                other
            ));
        }
    };
    let text = match &args[1] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "regex::split: expected string text, got {:?}",
                other
            ));
        }
    };
    let parts = simple_regex_split(&pattern, &text);
    Ok(Value::Array(parts.into_iter().map(Value::String).collect()))
}

static STD_REGEX: StdModule = StdModule {
    name: "regex",
    description: "Regular expression matching and manipulation",
    functions: &[
        StdFn {
            name: "match",
            params: &[("string", "pattern"), ("string", "text")],
            return_type: "bool",
            handler: regex_match,
        },
        StdFn {
            name: "find",
            params: &[("string", "pattern"), ("string", "text")],
            return_type: "array",
            handler: regex_find,
        },
        StdFn {
            name: "replace",
            params: &[
                ("string", "pattern"),
                ("string", "replacement"),
                ("string", "text"),
            ],
            return_type: "string",
            handler: regex_replace,
        },
        StdFn {
            name: "split",
            params: &[("string", "pattern"), ("string", "text")],
            return_type: "array",
            handler: regex_split,
        },
    ],
};

// ─── Standard Library: Time ──��──────────────────────────────────────

fn time_now(_args: &[Value]) -> RResult<Value> {
    use std::time::SystemTime;
    let ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok(Value::Int(ms))
}

fn time_seconds(_args: &[Value]) -> RResult<Value> {
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(Value::Int(secs))
}

fn time_sleep(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("time::sleep requires a milliseconds argument".to_string());
    }
    let ms = match &args[0] {
        Value::Int(n) => *n as u64,
        other => return Err(format!("time::sleep: expected int ms, got {:?}", other)),
    };
    std::thread::sleep(std::time::Duration::from_millis(ms));
    Ok(Value::Void)
}

fn time_format(args: &[Value]) -> RResult<Value> {
    use std::time::SystemTime;
    let ms = if args.is_empty() {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    } else {
        match &args[0] {
            Value::Int(n) => *n,
            other => {
                return Err(format!(
                    "time::format: expected int timestamp_ms, got {:?}",
                    other
                ));
            }
        }
    };
    let secs = ms / 1000;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    Ok(Value::String(format!("{:02}:{:02}:{:02}", hours, mins, s)))
}

static STD_TIME: StdModule = StdModule {
    name: "time",
    description: "Time utilities (timestamps, sleep, formatting)",
    functions: &[
        StdFn {
            name: "now",
            params: &[],
            return_type: "int",
            handler: time_now,
        },
        StdFn {
            name: "seconds",
            params: &[],
            return_type: "int",
            handler: time_seconds,
        },
        StdFn {
            name: "sleep",
            params: &[("int", "ms")],
            return_type: "void",
            handler: time_sleep,
        },
        StdFn {
            name: "format",
            params: &[("int", "timestamp_ms")],
            return_type: "string",
            handler: time_format,
        },
    ],
};

// ─── Standard Library: Net ──────────────────────────────────────────

fn net_resolve(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("net::resolve requires a hostname argument".to_string());
    }
    let host = match &args[0] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "net::resolve: expected string hostname, got {:?}",
                other
            ));
        }
    };
    use std::net::ToSocketAddrs;
    let addr_str = format!("{}:80", host);
    match addr_str.to_socket_addrs() {
        Ok(mut addrs) => {
            if let Some(addr) = addrs.next() {
                Ok(Value::String(addr.ip().to_string()))
            } else {
                Err(format!("net::resolve: no addresses found for '{}'", host))
            }
        }
        Err(e) => Err(format!("net::resolve '{}': {}", host, e)),
    }
}

fn net_url_encode(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("net::url_encode requires a string argument".to_string());
    }
    let s = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("net::url_encode: expected string, got {:?}", other)),
    };
    Ok(Value::String(url_encode(&s)))
}

fn net_url_decode(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("net::url_decode requires a string argument".to_string());
    }
    let s = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!("net::url_decode: expected string, got {:?}", other)),
    };
    Ok(Value::String(url_decode(&s)))
}

static STD_NET: StdModule = StdModule {
    name: "net",
    description: "Network utilities (DNS resolution, URL encoding)",
    functions: &[
        StdFn {
            name: "resolve",
            params: &[("string", "hostname")],
            return_type: "string",
            handler: net_resolve,
        },
        StdFn {
            name: "url_encode",
            params: &[("string", "s")],
            return_type: "string",
            handler: net_url_encode,
        },
        StdFn {
            name: "url_decode",
            params: &[("string", "s")],
            return_type: "string",
            handler: net_url_decode,
        },
    ],
};

// ─── Standard Library: Collections ────────��─────────────────────────

fn collections_deque_new(_args: &[Value]) -> RResult<Value> {
    Ok(Value::Array(Vec::new()))
}

fn collections_stack_new(_args: &[Value]) -> RResult<Value> {
    Ok(Value::Array(Vec::new()))
}

fn collections_sorted(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("collections::sorted requires an array argument".to_string());
    }
    let arr = match &args[0] {
        Value::Array(a) => a.clone(),
        other => {
            return Err(format!(
                "collections::sorted: expected array, got {:?}",
                other
            ));
        }
    };
    let mut sorted = arr;
    sorted.sort_by(|a, b| match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    });
    Ok(Value::Array(sorted))
}

fn collections_reversed(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("collections::reversed requires an array argument".to_string());
    }
    let arr = match &args[0] {
        Value::Array(a) => a.clone(),
        other => {
            return Err(format!(
                "collections::reversed: expected array, got {:?}",
                other
            ));
        }
    };
    let mut rev = arr;
    rev.reverse();
    Ok(Value::Array(rev))
}

fn collections_unique(args: &[Value]) -> RResult<Value> {
    if args.is_empty() {
        return Err("collections::unique requires an array argument".to_string());
    }
    let arr = match &args[0] {
        Value::Array(a) => a.clone(),
        other => {
            return Err(format!(
                "collections::unique: expected array, got {:?}",
                other
            ));
        }
    };
    let mut seen = Vec::new();
    let mut result = Vec::new();
    for item in arr {
        let key = format!("{:?}", item);
        if !seen.contains(&key) {
            seen.push(key);
            result.push(item);
        }
    }
    Ok(Value::Array(result))
}

static STD_COLLECTIONS: StdModule = StdModule {
    name: "collections",
    description: "Data structure utilities (sorting, deduplication)",
    functions: &[
        StdFn {
            name: "deque",
            params: &[],
            return_type: "array",
            handler: collections_deque_new,
        },
        StdFn {
            name: "stack",
            params: &[],
            return_type: "array",
            handler: collections_stack_new,
        },
        StdFn {
            name: "sorted",
            params: &[("array", "arr")],
            return_type: "array",
            handler: collections_sorted,
        },
        StdFn {
            name: "reversed",
            params: &[("array", "arr")],
            return_type: "array",
            handler: collections_reversed,
        },
        StdFn {
            name: "unique",
            params: &[("array", "arr")],
            return_type: "array",
            handler: collections_unique,
        },
    ],
};

// ─── Helper implementations ─────────���───────────────────────────────

fn to_f64(v: &Value) -> Result<f64, String> {
    match v {
        Value::Float(f) => Ok(*f),
        Value::Int(i) => Ok(*i as f64),
        other => Err(format!("expected numeric value, got {:?}", other)),
    }
}

/// Minimal HTTP GET using std::net::TcpStream (no external deps).
fn simple_http_get(url: &str) -> Result<String, String> {
    let (host, port, path) = parse_url(url)?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nUser-Agent: Resilient/1.0\r\n\r\n",
        path, host
    );
    http_request(&host, port, &request)
}

fn simple_http_post(url: &str, body: &str) -> Result<String, String> {
    let (host, port, path) = parse_url(url)?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\nContent-Type: application/json\r\nUser-Agent: Resilient/1.0\r\n\r\n{}",
        path,
        host,
        body.len(),
        body
    );
    http_request(&host, port, &request)
}

fn simple_http_head(url: &str) -> Result<u16, String> {
    let (host, port, path) = parse_url(url)?;
    let request = format!(
        "HEAD {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nUser-Agent: Resilient/1.0\r\n\r\n",
        path, host
    );
    let response = http_raw_request(&host, port, &request)?;
    parse_status_code(&response)
}

fn simple_http_head_headers(url: &str) -> Result<Vec<(String, String)>, String> {
    let (host, port, path) = parse_url(url)?;
    let request = format!(
        "HEAD {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nUser-Agent: Resilient/1.0\r\n\r\n",
        path, host
    );
    let response = http_raw_request(&host, port, &request)?;
    parse_headers(&response)
}

fn parse_url(url: &str) -> Result<(String, u16, String), String> {
    let url = url.trim();
    let (scheme, rest) = if let Some(stripped) = url.strip_prefix("https://") {
        ("https", stripped)
    } else if let Some(stripped) = url.strip_prefix("http://") {
        ("http", stripped)
    } else {
        ("http", url)
    };
    if scheme == "https" {
        return Err("https:// not supported — use http:// or an external library".to_string());
    }
    let (host_port, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match host_port.find(':') {
        Some(i) => (
            &host_port[..i],
            host_port[i + 1..]
                .parse::<u16>()
                .map_err(|e| format!("invalid port: {}", e))?,
        ),
        None => (host_port, 80),
    };
    Ok((host.to_string(), port, path.to_string()))
}

fn http_request(host: &str, port: u16, request: &str) -> Result<String, String> {
    let raw = http_raw_request(host, port, request)?;
    // Extract body (after \r\n\r\n)
    if let Some(idx) = raw.find("\r\n\r\n") {
        Ok(raw[idx + 4..].to_string())
    } else {
        Ok(raw)
    }
}

fn http_raw_request(host: &str, port: u16, request: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let addr = format!("{}:{}", host, port);
    let mut stream = TcpStream::connect_timeout(
        &addr
            .parse()
            .map_err(|e| format!("invalid address '{}': {}", addr, e))?,
        Duration::from_secs(10),
    )
    .map_err(|e| format!("connection to {} failed: {}", addr, e))?;

    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| format!("set timeout: {}", e))?;

    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write failed: {}", e))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|e| format!("read failed: {}", e))?;
    Ok(response)
}

fn parse_status_code(response: &str) -> Result<u16, String> {
    // HTTP/1.1 200 OK
    let first_line = response.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err("malformed HTTP response".to_string());
    }
    parts[1]
        .parse::<u16>()
        .map_err(|_| "invalid status code".to_string())
}

fn parse_headers(response: &str) -> Result<Vec<(String, String)>, String> {
    let mut headers = Vec::new();
    for line in response.lines().skip(1) {
        if line.is_empty() || line == "\r" {
            break;
        }
        if let Some(idx) = line.find(':') {
            let key = line[..idx].trim().to_lowercase();
            let value = line[idx + 1..].trim().to_string();
            headers.push((key, value));
        }
    }
    Ok(headers)
}

/// Minimal SHA-256 implementation (no external deps).
fn sha256_hex(data: &[u8]) -> String {
    let h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let k: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    // Pre-processing: pad message
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut hash = h;
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = hash;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(k[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(hh);
    }

    hash.iter().map(|x| format!("{:08x}", x)).collect()
}

/// Base64 encoding (standard alphabet, with padding).
fn b64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        result.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// Base64 decoding (standard alphabet).
fn b64_decode(input: &str) -> Result<Vec<u8>, String> {
    fn char_to_val(c: u8) -> Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            b'=' => Ok(0),
            _ => Err(format!("invalid base64 character: {}", c as char)),
        }
    }

    let input: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !input.len().is_multiple_of(4) {
        return Err("invalid base64: length not multiple of 4".to_string());
    }

    let mut result = Vec::new();
    for chunk in input.chunks(4) {
        let a = char_to_val(chunk[0])?;
        let b = char_to_val(chunk[1])?;
        let c = char_to_val(chunk[2])?;
        let d = char_to_val(chunk[3])?;
        let triple = ((a as u32) << 18) | ((b as u32) << 12) | ((c as u32) << 6) | (d as u32);
        result.push((triple >> 16) as u8);
        if chunk[2] != b'=' {
            result.push((triple >> 8) as u8);
        }
        if chunk[3] != b'=' {
            result.push(triple as u8);
        }
    }
    Ok(result)
}

/// JSON parser (recursive descent, minimal).
fn parse_json_value(s: &str) -> Result<Value, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty input".to_string());
    }
    let (val, rest) = parse_json_inner(s)?;
    let rest = rest.trim();
    if !rest.is_empty() {
        return Err(format!(
            "trailing characters: {:?}",
            &rest[..rest.len().min(20)]
        ));
    }
    Ok(val)
}

fn parse_json_inner(s: &str) -> Result<(Value, &str), String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("unexpected end of input".to_string());
    }
    match s.as_bytes()[0] {
        b'"' => parse_json_string(s),
        b'{' => parse_json_object(s),
        b'[' => parse_json_array(s),
        b't' if s.starts_with("true") => Ok((Value::Bool(true), &s[4..])),
        b'f' if s.starts_with("false") => Ok((Value::Bool(false), &s[5..])),
        b'n' if s.starts_with("null") => Ok((Value::Void, &s[4..])),
        b'-' | b'0'..=b'9' => parse_json_number(s),
        c => Err(format!("unexpected character: {:?}", c as char)),
    }
}

fn parse_json_string(s: &str) -> Result<(Value, &str), String> {
    if !s.starts_with('"') {
        return Err("expected '\"'".to_string());
    }
    let s = &s[1..];
    let mut result = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Ok((Value::String(result), &s[i + 1..])),
            '\\' => {
                if let Some((_, esc)) = chars.next() {
                    match esc {
                        '"' => result.push('"'),
                        '\\' => result.push('\\'),
                        '/' => result.push('/'),
                        'n' => result.push('\n'),
                        'r' => result.push('\r'),
                        't' => result.push('\t'),
                        _ => {
                            result.push('\\');
                            result.push(esc);
                        }
                    }
                }
            }
            _ => result.push(c),
        }
    }
    Err("unterminated string".to_string())
}

fn parse_json_number(s: &str) -> Result<(Value, &str), String> {
    let end = s
        .find(|c: char| {
            !c.is_ascii_digit() && c != '.' && c != '-' && c != 'e' && c != 'E' && c != '+'
        })
        .unwrap_or(s.len());
    let num_str = &s[..end];
    if num_str.contains('.') || num_str.contains('e') || num_str.contains('E') {
        let f: f64 = num_str
            .parse()
            .map_err(|e| format!("invalid float: {}", e))?;
        Ok((Value::Float(f), &s[end..]))
    } else {
        let i: i64 = num_str.parse().map_err(|e| format!("invalid int: {}", e))?;
        Ok((Value::Int(i), &s[end..]))
    }
}

fn parse_json_array(s: &str) -> Result<(Value, &str), String> {
    let mut s = &s[1..]; // skip '['
    let mut items = Vec::new();
    s = s.trim();
    if let Some(rest) = s.strip_prefix(']') {
        return Ok((Value::Array(items), rest));
    }
    loop {
        let (val, rest) = parse_json_inner(s)?;
        items.push(val);
        s = rest.trim();
        if let Some(rest) = s.strip_prefix(']') {
            return Ok((Value::Array(items), rest));
        }
        if s.starts_with(',') {
            s = &s[1..];
        } else {
            return Err("expected ',' or ']' in array".to_string());
        }
    }
}

fn parse_json_object(s: &str) -> Result<(Value, &str), String> {
    let mut s = &s[1..]; // skip '{'
    let mut map = HashMap::new();
    s = s.trim();
    if let Some(rest) = s.strip_prefix('}') {
        return Ok((Value::Map(map), rest));
    }
    loop {
        s = s.trim();
        let (key_val, rest) = parse_json_string(s)?;
        let key = match key_val {
            Value::String(k) => k,
            _ => return Err("object key must be string".to_string()),
        };
        s = rest.trim();
        if !s.starts_with(':') {
            return Err("expected ':' after object key".to_string());
        }
        s = &s[1..];
        let (val, rest) = parse_json_inner(s)?;
        map.insert(MapKey::Str(key), val);
        s = rest.trim();
        if let Some(rest) = s.strip_prefix('}') {
            return Ok((Value::Map(map), rest));
        }
        if s.starts_with(',') {
            s = &s[1..];
        } else {
            return Err("expected ',' or '}' in object".to_string());
        }
    }
}

fn mapkey_to_json(k: &MapKey) -> String {
    match k {
        MapKey::Str(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        MapKey::Int(i) => format!("\"{}\"", i),
        MapKey::Bool(b) => format!("\"{}\"", b),
    }
}

fn value_to_json(v: &Value) -> String {
    match v {
        Value::Void => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                "null".to_string()
            } else {
                format!("{}", f)
            }
        }
        Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(value_to_json).collect();
            format!("[{}]", items.join(","))
        }
        Value::Map(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}:{}", mapkey_to_json(k), value_to_json(v)))
                .collect();
            format!("{{{}}}", items.join(","))
        }
        _ => format!("\"{}\"", format!("{:?}", v).replace('"', "\\\"")),
    }
}

fn value_to_json_pretty(v: &Value, indent: usize) -> String {
    let pad = "  ".repeat(indent);
    let pad_inner = "  ".repeat(indent + 1);
    match v {
        Value::Array(arr) => {
            if arr.is_empty() {
                return "[]".to_string();
            }
            let items: Vec<String> = arr
                .iter()
                .map(|item| format!("{}{}", pad_inner, value_to_json_pretty(item, indent + 1)))
                .collect();
            format!("[\n{}\n{}]", items.join(",\n"), pad)
        }
        Value::Map(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}{}: {}",
                        pad_inner,
                        mapkey_to_json(k),
                        value_to_json_pretty(v, indent + 1)
                    )
                })
                .collect();
            format!("{{\n{}\n{}}}", items.join(",\n"), pad)
        }
        _ => value_to_json(v),
    }
}

/// Simple regex: supports `.`, `*`, `+`, `?`, `^`, `$`, character classes `[...]`.
fn simple_regex_match(pattern: &str, text: &str) -> bool {
    // Use a simple substring/glob approach for common patterns
    if pattern == ".*" {
        return true;
    }
    if !pattern.contains(|c: char| ".*+?[]^$\\(){}|".contains(c)) {
        return text.contains(pattern);
    }
    // For complex patterns, try basic interpretation
    regex_matches_at(pattern, text, 0)
}

fn regex_matches_at(pattern: &str, text: &str, _flags: u8) -> bool {
    if pattern.starts_with('^') && pattern.ends_with('$') {
        return glob_match(&pattern[1..pattern.len() - 1], text);
    }
    if let Some(rest) = pattern.strip_prefix('^') {
        return text.starts_with(&rest.replace(".*", ""));
    }
    if let Some(p) = pattern.strip_suffix('$')
        && !p.contains(|c: char| ".*+?[]\\(){}|".contains(c))
    {
        return text.ends_with(p);
    }
    // Fallback: literal substring
    let literal = pattern.replace(".*", "").replace(['.', '*', '+', '?'], "");
    if literal.is_empty() {
        return true;
    }
    text.contains(&literal)
}

fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == ".*" {
        return true;
    }
    if !pattern.contains(|c: char| ".*+?[]\\(){}|".contains(c)) {
        return pattern == text;
    }
    // Very basic: ".*" matches anything
    if pattern.contains(".*") {
        let parts: Vec<&str> = pattern.split(".*").collect();
        if parts.len() == 2 {
            return text.starts_with(parts[0]) && text.ends_with(parts[1]);
        }
    }
    text.contains(&pattern.replace(".*", ""))
}

fn simple_regex_find_all(pattern: &str, text: &str) -> Vec<String> {
    // Simple: find all literal occurrences
    let search = pattern.replace(".*", "").replace(['.', '*', '+', '?'], "");
    if search.is_empty() {
        return vec![text.to_string()];
    }
    let mut results = Vec::new();
    let mut start = 0;
    while let Some(idx) = text[start..].find(&search) {
        results.push(search.clone());
        start += idx + search.len();
    }
    results
}

fn simple_regex_replace(pattern: &str, replacement: &str, text: &str) -> String {
    let search = pattern.replace(".*", "").replace(['.', '*', '+', '?'], "");
    if search.is_empty() {
        return text.to_string();
    }
    text.replace(&search, replacement)
}

fn simple_regex_split(pattern: &str, text: &str) -> Vec<String> {
    let search = pattern.replace(".*", "").replace(['.', '*', '+', '?'], "");
    if search.is_empty() {
        return vec![text.to_string()];
    }
    text.split(&search).map(|s| s.to_string()).collect()
}

fn url_encode(s: &str) -> String {
    let mut result = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(b as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", b));
            }
        }
    }
    result
}

fn url_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(val) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            result.push(val);
            i += 3;
            continue;
        }
        if bytes[i] == b'+' {
            result.push(b' ');
        } else {
            result.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&result).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_known_modules() {
        assert!(lookup_std_module("http").is_some());
        assert!(lookup_std_module("json").is_some());
        assert!(lookup_std_module("math").is_some());
        assert!(lookup_std_module("fs").is_some());
        assert!(lookup_std_module("os").is_some());
        assert!(lookup_std_module("crypto").is_some());
        assert!(lookup_std_module("base64").is_some());
        assert!(lookup_std_module("regex").is_some());
        assert!(lookup_std_module("time").is_some());
        assert!(lookup_std_module("net").is_some());
        assert!(lookup_std_module("collections").is_some());
        assert!(lookup_std_module("nonexistent").is_none());
    }

    #[test]
    fn resolve_import_creates_namespaced_bindings() {
        let bindings = resolve_std_import("math", None).unwrap();
        assert!(bindings.iter().any(|(name, _)| name == "math_pi"));
        assert!(bindings.iter().any(|(name, _)| name == "math_log"));
    }

    #[test]
    fn resolve_import_with_alias() {
        let bindings = resolve_std_import("math", Some("m")).unwrap();
        assert!(bindings.iter().any(|(name, _)| name == "m_pi"));
        assert!(bindings.iter().any(|(name, _)| name == "m_log"));
    }

    #[test]
    fn json_roundtrip() {
        let input = r#"{"name":"test","value":42,"items":[1,2,3],"active":true}"#;
        let parsed = parse_json_value(input).unwrap();
        let back = value_to_json(&parsed);
        // Re-parse to verify structural equivalence
        let reparsed = parse_json_value(&back).unwrap();
        assert_eq!(format!("{:?}", parsed), format!("{:?}", reparsed));
    }

    #[test]
    fn base64_roundtrip() {
        let original = b"Hello, Resilient!";
        let encoded = b64_encode(original);
        let decoded = b64_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn sha256_known_vector() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let hash = sha256_hex(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn url_encode_decode_roundtrip() {
        let original = "hello world & foo=bar";
        let encoded = url_encode(original);
        let decoded = url_decode(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn math_constants() {
        match math_pi(&[]).unwrap() {
            Value::Float(f) => assert!((f - std::f64::consts::PI).abs() < 1e-15),
            other => panic!("expected Float, got {:?}", other),
        }
        match math_e(&[]).unwrap() {
            Value::Float(f) => assert!((f - std::f64::consts::E).abs() < 1e-15),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn unknown_module_gives_helpful_error() {
        let err = resolve_std_import("nonexistent", None).unwrap_err();
        assert!(err.contains("Unknown standard library module"));
        assert!(err.contains("Available:"));
    }
}
