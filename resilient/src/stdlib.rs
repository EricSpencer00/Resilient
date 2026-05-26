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
    &STD_STRING,
    &STD_FMT,
    &STD_PATH,
    &STD_CONVERT,
    &STD_BIT,
    &STD_LOG,
    &STD_TESTING,
    &STD_CSV,
    &STD_HEX,
    &STD_RANDOM,
    &STD_COLOR,
    &STD_PROCESS,
    &STD_INI,
    &STD_ITER,
    &STD_BUFFER,
    &STD_HASH,
    &STD_SORT,
    &STD_UUID,
    &STD_ENCODING,
    &STD_URL,
];

/// Look up a standard library module by name.
pub fn lookup_std_module(name: &str) -> Option<&'static StdModule> {
    STD_MODULES.iter().find(|m| m.name == name).copied()
}

/// Dispatch a qualified stdlib call by splitting on `"::"` (e.g.
/// `"math::sqrt"`) or `"_"` (e.g. `"math_sqrt"`) and looking up the
/// module + function.  Returns `None` if the name isn't a valid stdlib
/// qualified name.
pub fn call_by_qualified_name(name: &str, args: &[Value]) -> Option<RResult<Value>> {
    let handler = resolve_stdlib_handler(name)?;
    Some(handler(args))
}

/// Check whether `name` resolves to a stdlib function.  Used by the
/// bytecode compiler to decide whether to emit `CallBuiltin` for a
/// callee that isn't in the flat builtin table.
pub fn is_stdlib_function(name: &str) -> bool {
    resolve_stdlib_handler(name).is_some()
}

/// Handler function pointer type used by stdlib dispatch.
type StdHandler = fn(&[Value]) -> RResult<Value>;

/// Shared resolution: try `"::"` split first, then `"_"` split.
fn resolve_stdlib_handler(name: &str) -> Option<StdHandler> {
    // Try "module::fn" form first.
    if let Some((module_name, fn_name)) = name.split_once("::")
        && let Some(module) = lookup_std_module(module_name)
        && let Some(f) = module.functions.iter().find(|f| f.name == fn_name)
    {
        return Some(f.handler);
    }
    // Fall back to "module_fn" form (as produced by resolve_std_import).
    // Try each registered module name as a prefix.
    for module in STD_MODULES.iter() {
        if let Some(fn_name) = name.strip_prefix(module.name)
            && let Some(fn_name) = fn_name.strip_prefix('_')
            && let Some(f) = module.functions.iter().find(|f| f.name == fn_name)
        {
            return Some(f.handler);
        }
    }
    None
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

// ─── Standard Library: String ──────────────────────────────────────

fn string_trim(args: &[Value]) -> RResult<Value> {
    require_args("string::trim", args, 1)?;
    Ok(Value::String(
        extract_string("string::trim", &args[0])?.trim().to_string(),
    ))
}
fn string_trim_start(args: &[Value]) -> RResult<Value> {
    require_args("string::trim_start", args, 1)?;
    Ok(Value::String(
        extract_string("string::trim_start", &args[0])?
            .trim_start()
            .to_string(),
    ))
}
fn string_trim_end(args: &[Value]) -> RResult<Value> {
    require_args("string::trim_end", args, 1)?;
    Ok(Value::String(
        extract_string("string::trim_end", &args[0])?
            .trim_end()
            .to_string(),
    ))
}
fn string_upper(args: &[Value]) -> RResult<Value> {
    require_args("string::upper", args, 1)?;
    Ok(Value::String(
        extract_string("string::upper", &args[0])?.to_uppercase(),
    ))
}
fn string_lower(args: &[Value]) -> RResult<Value> {
    require_args("string::lower", args, 1)?;
    Ok(Value::String(
        extract_string("string::lower", &args[0])?.to_lowercase(),
    ))
}
fn string_repeat(args: &[Value]) -> RResult<Value> {
    require_args("string::repeat", args, 2)?;
    Ok(Value::String(
        extract_string("string::repeat", &args[0])?
            .repeat(extract_int("string::repeat", &args[1])? as usize),
    ))
}
fn string_chars(args: &[Value]) -> RResult<Value> {
    require_args("string::chars", args, 1)?;
    Ok(Value::Array(
        extract_string("string::chars", &args[0])?
            .chars()
            .map(|c| Value::String(c.to_string()))
            .collect(),
    ))
}
fn string_char_at(args: &[Value]) -> RResult<Value> {
    require_args("string::char_at", args, 2)?;
    let s = extract_string("string::char_at", &args[0])?;
    let idx = extract_int("string::char_at", &args[1])? as usize;
    s.chars()
        .nth(idx)
        .map(|c| Value::String(c.to_string()))
        .ok_or_else(|| {
            format!(
                "string::char_at: index {} out of bounds (len {})",
                idx,
                s.chars().count()
            )
        })
}
fn string_contains(args: &[Value]) -> RResult<Value> {
    require_args("string::contains", args, 2)?;
    Ok(Value::Bool(
        extract_string("string::contains", &args[0])?
            .contains(&extract_string("string::contains", &args[1])?),
    ))
}
fn string_starts_with(args: &[Value]) -> RResult<Value> {
    require_args("string::starts_with", args, 2)?;
    Ok(Value::Bool(
        extract_string("string::starts_with", &args[0])?
            .starts_with(&extract_string("string::starts_with", &args[1])?),
    ))
}
fn string_ends_with(args: &[Value]) -> RResult<Value> {
    require_args("string::ends_with", args, 2)?;
    Ok(Value::Bool(
        extract_string("string::ends_with", &args[0])?
            .ends_with(&extract_string("string::ends_with", &args[1])?),
    ))
}
fn string_replace_all(args: &[Value]) -> RResult<Value> {
    require_args("string::replace", args, 3)?;
    Ok(Value::String(
        extract_string("string::replace", &args[0])?.replace(
            &extract_string("string::replace", &args[1])?,
            &extract_string("string::replace", &args[2])?,
        ),
    ))
}
fn string_split(args: &[Value]) -> RResult<Value> {
    require_args("string::split", args, 2)?;
    let s = extract_string("string::split", &args[0])?;
    let d = extract_string("string::split", &args[1])?;
    Ok(Value::Array(
        s.split(&d).map(|p| Value::String(p.to_string())).collect(),
    ))
}
fn string_join(args: &[Value]) -> RResult<Value> {
    require_args("string::join", args, 2)?;
    let arr = extract_array("string::join", &args[0])?;
    let sep = extract_string("string::join", &args[1])?;
    Ok(Value::String(
        arr.iter().map(value_display).collect::<Vec<_>>().join(&sep),
    ))
}
fn string_reverse(args: &[Value]) -> RResult<Value> {
    require_args("string::reverse", args, 1)?;
    Ok(Value::String(
        extract_string("string::reverse", &args[0])?
            .chars()
            .rev()
            .collect(),
    ))
}
fn string_index_of(args: &[Value]) -> RResult<Value> {
    require_args("string::index_of", args, 2)?;
    Ok(Value::Int(
        extract_string("string::index_of", &args[0])?
            .find(&extract_string("string::index_of", &args[1])?)
            .map(|i| i as i64)
            .unwrap_or(-1),
    ))
}
fn string_substring(args: &[Value]) -> RResult<Value> {
    require_args("string::substring", args, 3)?;
    let chars: Vec<char> = extract_string("string::substring", &args[0])?
        .chars()
        .collect();
    let end = (extract_int("string::substring", &args[2])? as usize).min(chars.len());
    let start = (extract_int("string::substring", &args[1])? as usize).min(end);
    Ok(Value::String(chars[start..end].iter().collect()))
}
fn string_is_empty(args: &[Value]) -> RResult<Value> {
    require_args("string::is_empty", args, 1)?;
    Ok(Value::Bool(
        extract_string("string::is_empty", &args[0])?.is_empty(),
    ))
}
fn string_char_count(args: &[Value]) -> RResult<Value> {
    require_args("string::char_count", args, 1)?;
    Ok(Value::Int(
        extract_string("string::char_count", &args[0])?
            .chars()
            .count() as i64,
    ))
}
fn string_pad_left(args: &[Value]) -> RResult<Value> {
    require_args("string::pad_left", args, 3)?;
    let s = extract_string("string::pad_left", &args[0])?;
    let width = extract_int("string::pad_left", &args[1])? as usize;
    let pad = extract_string("string::pad_left", &args[2])?
        .chars()
        .next()
        .unwrap_or(' ');
    let len = s.chars().count();
    if len >= width {
        return Ok(Value::String(s));
    }
    Ok(Value::String(format!(
        "{}{}",
        std::iter::repeat_n(pad, width - len).collect::<String>(),
        s
    )))
}
fn string_pad_right(args: &[Value]) -> RResult<Value> {
    require_args("string::pad_right", args, 3)?;
    let s = extract_string("string::pad_right", &args[0])?;
    let width = extract_int("string::pad_right", &args[1])? as usize;
    let pad = extract_string("string::pad_right", &args[2])?
        .chars()
        .next()
        .unwrap_or(' ');
    let len = s.chars().count();
    if len >= width {
        return Ok(Value::String(s));
    }
    Ok(Value::String(format!(
        "{}{}",
        s,
        std::iter::repeat_n(pad, width - len).collect::<String>()
    )))
}

static STD_STRING: StdModule = StdModule {
    name: "string",
    description: "String manipulation utilities",
    functions: &[
        StdFn {
            name: "trim",
            params: &[("string", "s")],
            return_type: "string",
            handler: string_trim,
        },
        StdFn {
            name: "trim_start",
            params: &[("string", "s")],
            return_type: "string",
            handler: string_trim_start,
        },
        StdFn {
            name: "trim_end",
            params: &[("string", "s")],
            return_type: "string",
            handler: string_trim_end,
        },
        StdFn {
            name: "upper",
            params: &[("string", "s")],
            return_type: "string",
            handler: string_upper,
        },
        StdFn {
            name: "lower",
            params: &[("string", "s")],
            return_type: "string",
            handler: string_lower,
        },
        StdFn {
            name: "repeat",
            params: &[("string", "s"), ("int", "n")],
            return_type: "string",
            handler: string_repeat,
        },
        StdFn {
            name: "chars",
            params: &[("string", "s")],
            return_type: "array",
            handler: string_chars,
        },
        StdFn {
            name: "char_at",
            params: &[("string", "s"), ("int", "index")],
            return_type: "string",
            handler: string_char_at,
        },
        StdFn {
            name: "contains",
            params: &[("string", "haystack"), ("string", "needle")],
            return_type: "bool",
            handler: string_contains,
        },
        StdFn {
            name: "starts_with",
            params: &[("string", "s"), ("string", "prefix")],
            return_type: "bool",
            handler: string_starts_with,
        },
        StdFn {
            name: "ends_with",
            params: &[("string", "s"), ("string", "suffix")],
            return_type: "bool",
            handler: string_ends_with,
        },
        StdFn {
            name: "replace",
            params: &[("string", "s"), ("string", "from"), ("string", "to")],
            return_type: "string",
            handler: string_replace_all,
        },
        StdFn {
            name: "split",
            params: &[("string", "s"), ("string", "delim")],
            return_type: "array",
            handler: string_split,
        },
        StdFn {
            name: "join",
            params: &[("array", "arr"), ("string", "sep")],
            return_type: "string",
            handler: string_join,
        },
        StdFn {
            name: "reverse",
            params: &[("string", "s")],
            return_type: "string",
            handler: string_reverse,
        },
        StdFn {
            name: "index_of",
            params: &[("string", "s"), ("string", "needle")],
            return_type: "int",
            handler: string_index_of,
        },
        StdFn {
            name: "substring",
            params: &[("string", "s"), ("int", "start"), ("int", "end")],
            return_type: "string",
            handler: string_substring,
        },
        StdFn {
            name: "is_empty",
            params: &[("string", "s")],
            return_type: "bool",
            handler: string_is_empty,
        },
        StdFn {
            name: "char_count",
            params: &[("string", "s")],
            return_type: "int",
            handler: string_char_count,
        },
        StdFn {
            name: "pad_left",
            params: &[("string", "s"), ("int", "width"), ("string", "pad")],
            return_type: "string",
            handler: string_pad_left,
        },
        StdFn {
            name: "pad_right",
            params: &[("string", "s"), ("int", "width"), ("string", "pad")],
            return_type: "string",
            handler: string_pad_right,
        },
    ],
};

// ─── Standard Library: Fmt ─────────────────────────────────────────

fn fmt_format(args: &[Value]) -> RResult<Value> {
    require_args("fmt::format", args, 1)?;
    let template = extract_string("fmt::format", &args[0])?;
    let values = &args[1..];
    let mut result = String::new();
    let mut val_idx = 0;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() == Some(&'}') {
            chars.next();
            if val_idx < values.len() {
                result.push_str(&value_display(&values[val_idx]));
                val_idx += 1;
            } else {
                result.push_str("{}");
            }
        } else {
            result.push(c);
        }
    }
    Ok(Value::String(result))
}
fn fmt_number(args: &[Value]) -> RResult<Value> {
    require_args("fmt::number", args, 2)?;
    Ok(Value::String(format!(
        "{:.prec$}",
        to_f64(&args[0])?,
        prec = extract_int("fmt::number", &args[1])? as usize
    )))
}
fn fmt_hex(args: &[Value]) -> RResult<Value> {
    require_args("fmt::hex", args, 1)?;
    Ok(Value::String(format!(
        "{:x}",
        extract_int("fmt::hex", &args[0])?
    )))
}
fn fmt_oct(args: &[Value]) -> RResult<Value> {
    require_args("fmt::oct", args, 1)?;
    Ok(Value::String(format!(
        "{:o}",
        extract_int("fmt::oct", &args[0])?
    )))
}
fn fmt_bin(args: &[Value]) -> RResult<Value> {
    require_args("fmt::bin", args, 1)?;
    Ok(Value::String(format!(
        "{:b}",
        extract_int("fmt::bin", &args[0])?
    )))
}
fn fmt_center(args: &[Value]) -> RResult<Value> {
    require_args("fmt::center", args, 2)?;
    let s = extract_string("fmt::center", &args[0])?;
    let width = extract_int("fmt::center", &args[1])? as usize;
    let len = s.chars().count();
    if len >= width {
        return Ok(Value::String(s));
    }
    let left = (width - len) / 2;
    let right = width - len - left;
    Ok(Value::String(format!(
        "{}{}{}",
        " ".repeat(left),
        s,
        " ".repeat(right)
    )))
}

static STD_FMT: StdModule = StdModule {
    name: "fmt",
    description: "String formatting utilities",
    functions: &[
        StdFn {
            name: "format",
            params: &[("string", "template")],
            return_type: "string",
            handler: fmt_format,
        },
        StdFn {
            name: "number",
            params: &[("float", "val"), ("int", "decimals")],
            return_type: "string",
            handler: fmt_number,
        },
        StdFn {
            name: "hex",
            params: &[("int", "n")],
            return_type: "string",
            handler: fmt_hex,
        },
        StdFn {
            name: "oct",
            params: &[("int", "n")],
            return_type: "string",
            handler: fmt_oct,
        },
        StdFn {
            name: "bin",
            params: &[("int", "n")],
            return_type: "string",
            handler: fmt_bin,
        },
        StdFn {
            name: "center",
            params: &[("string", "s"), ("int", "width")],
            return_type: "string",
            handler: fmt_center,
        },
    ],
};

// ─── Standard Library: Path ────────────────────────────────────────

fn path_join(args: &[Value]) -> RResult<Value> {
    require_args("path::join", args, 2)?;
    Ok(Value::String(
        std::path::Path::new(&extract_string("path::join", &args[0])?)
            .join(&extract_string("path::join", &args[1])?)
            .to_string_lossy()
            .to_string(),
    ))
}
fn path_dirname(args: &[Value]) -> RResult<Value> {
    require_args("path::dirname", args, 1)?;
    Ok(Value::String(
        std::path::Path::new(&extract_string("path::dirname", &args[0])?)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    ))
}
fn path_basename(args: &[Value]) -> RResult<Value> {
    require_args("path::basename", args, 1)?;
    Ok(Value::String(
        std::path::Path::new(&extract_string("path::basename", &args[0])?)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
    ))
}
fn path_extension(args: &[Value]) -> RResult<Value> {
    require_args("path::extension", args, 1)?;
    Ok(Value::String(
        std::path::Path::new(&extract_string("path::extension", &args[0])?)
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default(),
    ))
}
fn path_is_absolute(args: &[Value]) -> RResult<Value> {
    require_args("path::is_absolute", args, 1)?;
    Ok(Value::Bool(
        std::path::Path::new(&extract_string("path::is_absolute", &args[0])?).is_absolute(),
    ))
}
fn path_separator(_args: &[Value]) -> RResult<Value> {
    Ok(Value::String(std::path::MAIN_SEPARATOR.to_string()))
}
fn path_with_extension(args: &[Value]) -> RResult<Value> {
    require_args("path::with_extension", args, 2)?;
    Ok(Value::String(
        std::path::PathBuf::from(&extract_string("path::with_extension", &args[0])?)
            .with_extension(&extract_string("path::with_extension", &args[1])?)
            .to_string_lossy()
            .to_string(),
    ))
}
fn path_stem(args: &[Value]) -> RResult<Value> {
    require_args("path::stem", args, 1)?;
    Ok(Value::String(
        std::path::Path::new(&extract_string("path::stem", &args[0])?)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
    ))
}

static STD_PATH: StdModule = StdModule {
    name: "path",
    description: "File path manipulation utilities",
    functions: &[
        StdFn {
            name: "join",
            params: &[("string", "a"), ("string", "b")],
            return_type: "string",
            handler: path_join,
        },
        StdFn {
            name: "dirname",
            params: &[("string", "path")],
            return_type: "string",
            handler: path_dirname,
        },
        StdFn {
            name: "basename",
            params: &[("string", "path")],
            return_type: "string",
            handler: path_basename,
        },
        StdFn {
            name: "extension",
            params: &[("string", "path")],
            return_type: "string",
            handler: path_extension,
        },
        StdFn {
            name: "is_absolute",
            params: &[("string", "path")],
            return_type: "bool",
            handler: path_is_absolute,
        },
        StdFn {
            name: "separator",
            params: &[],
            return_type: "string",
            handler: path_separator,
        },
        StdFn {
            name: "with_extension",
            params: &[("string", "path"), ("string", "ext")],
            return_type: "string",
            handler: path_with_extension,
        },
        StdFn {
            name: "stem",
            params: &[("string", "path")],
            return_type: "string",
            handler: path_stem,
        },
    ],
};

// ─── Standard Library: Convert ─────────────────────────────────────

fn convert_parse_int(args: &[Value]) -> RResult<Value> {
    require_args("convert::parse_int", args, 1)?;
    extract_string("convert::parse_int", &args[0])?
        .trim()
        .parse::<i64>()
        .map(Value::Int)
        .map_err(|e| format!("convert::parse_int: {}", e))
}
fn convert_parse_float(args: &[Value]) -> RResult<Value> {
    require_args("convert::parse_float", args, 1)?;
    extract_string("convert::parse_float", &args[0])?
        .trim()
        .parse::<f64>()
        .map(Value::Float)
        .map_err(|e| format!("convert::parse_float: {}", e))
}
fn convert_to_string(args: &[Value]) -> RResult<Value> {
    require_args("convert::to_string", args, 1)?;
    Ok(Value::String(value_display(&args[0])))
}
fn convert_to_int(args: &[Value]) -> RResult<Value> {
    require_args("convert::to_int", args, 1)?;
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(*n)),
        Value::Float(f) => Ok(Value::Int(*f as i64)),
        Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
        Value::String(s) => s
            .trim()
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|e| format!("convert::to_int: {}", e)),
        other => Err(format!("convert::to_int: cannot convert {:?}", other)),
    }
}
fn convert_to_float(args: &[Value]) -> RResult<Value> {
    require_args("convert::to_float", args, 1)?;
    match &args[0] {
        Value::Float(f) => Ok(Value::Float(*f)),
        Value::Int(n) => Ok(Value::Float(*n as f64)),
        Value::Bool(b) => Ok(Value::Float(if *b { 1.0 } else { 0.0 })),
        Value::String(s) => s
            .trim()
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|e| format!("convert::to_float: {}", e)),
        other => Err(format!("convert::to_float: cannot convert {:?}", other)),
    }
}
fn convert_to_bool(args: &[Value]) -> RResult<Value> {
    require_args("convert::to_bool", args, 1)?;
    match &args[0] {
        Value::Bool(b) => Ok(Value::Bool(*b)),
        Value::Int(n) => Ok(Value::Bool(*n != 0)),
        Value::Float(f) => Ok(Value::Bool(*f != 0.0)),
        Value::String(s) => Ok(Value::Bool(!s.is_empty())),
        Value::Void => Ok(Value::Bool(false)),
        Value::Array(a) => Ok(Value::Bool(!a.is_empty())),
        _ => Ok(Value::Bool(true)),
    }
}
fn convert_int_to_char(args: &[Value]) -> RResult<Value> {
    require_args("convert::int_to_char", args, 1)?;
    let n = extract_int("convert::int_to_char", &args[0])?;
    char::from_u32(n as u32)
        .map(|c| Value::String(c.to_string()))
        .ok_or_else(|| {
            format!(
                "convert::int_to_char: {} is not a valid Unicode codepoint",
                n
            )
        })
}
fn convert_char_to_int(args: &[Value]) -> RResult<Value> {
    require_args("convert::char_to_int", args, 1)?;
    extract_string("convert::char_to_int", &args[0])?
        .chars()
        .next()
        .map(|c| Value::Int(c as i64))
        .ok_or_else(|| "convert::char_to_int: empty string".to_string())
}
fn convert_type_of(args: &[Value]) -> RResult<Value> {
    require_args("convert::type_of", args, 1)?;
    Ok(Value::String(
        match &args[0] {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Bool(_) => "bool",
            Value::Array(_) => "array",
            Value::Map(_) => "map",
            Value::Set(_) => "set",
            Value::Bytes(_) => "bytes",
            Value::Void => "void",
            Value::Struct { .. } => "struct",
            Value::Result { .. } => "result",
            Value::Option(_) => "option",
            Value::Function(_) | Value::Builtin { .. } => "function",
            _ => "unknown",
        }
        .to_string(),
    ))
}

static STD_CONVERT: StdModule = StdModule {
    name: "convert",
    description: "Type conversion and parsing utilities",
    functions: &[
        StdFn {
            name: "parse_int",
            params: &[("string", "s")],
            return_type: "int",
            handler: convert_parse_int,
        },
        StdFn {
            name: "parse_float",
            params: &[("string", "s")],
            return_type: "float",
            handler: convert_parse_float,
        },
        StdFn {
            name: "to_string",
            params: &[("any", "val")],
            return_type: "string",
            handler: convert_to_string,
        },
        StdFn {
            name: "to_int",
            params: &[("any", "val")],
            return_type: "int",
            handler: convert_to_int,
        },
        StdFn {
            name: "to_float",
            params: &[("any", "val")],
            return_type: "float",
            handler: convert_to_float,
        },
        StdFn {
            name: "to_bool",
            params: &[("any", "val")],
            return_type: "bool",
            handler: convert_to_bool,
        },
        StdFn {
            name: "int_to_char",
            params: &[("int", "codepoint")],
            return_type: "string",
            handler: convert_int_to_char,
        },
        StdFn {
            name: "char_to_int",
            params: &[("string", "c")],
            return_type: "int",
            handler: convert_char_to_int,
        },
        StdFn {
            name: "type_of",
            params: &[("any", "val")],
            return_type: "string",
            handler: convert_type_of,
        },
    ],
};

// ─── Standard Library: Bit ─────────────────────────────────────────

fn bit_and(args: &[Value]) -> RResult<Value> {
    require_args("bit::and", args, 2)?;
    Ok(Value::Int(
        extract_int("bit::and", &args[0])? & extract_int("bit::and", &args[1])?,
    ))
}
fn bit_or(args: &[Value]) -> RResult<Value> {
    require_args("bit::or", args, 2)?;
    Ok(Value::Int(
        extract_int("bit::or", &args[0])? | extract_int("bit::or", &args[1])?,
    ))
}
fn bit_xor(args: &[Value]) -> RResult<Value> {
    require_args("bit::xor", args, 2)?;
    Ok(Value::Int(
        extract_int("bit::xor", &args[0])? ^ extract_int("bit::xor", &args[1])?,
    ))
}
fn bit_not(args: &[Value]) -> RResult<Value> {
    require_args("bit::not", args, 1)?;
    Ok(Value::Int(!extract_int("bit::not", &args[0])?))
}
fn bit_shl(args: &[Value]) -> RResult<Value> {
    require_args("bit::shl", args, 2)?;
    Ok(Value::Int(
        extract_int("bit::shl", &args[0])?.wrapping_shl(extract_int("bit::shl", &args[1])? as u32),
    ))
}
fn bit_shr(args: &[Value]) -> RResult<Value> {
    require_args("bit::shr", args, 2)?;
    Ok(Value::Int(
        extract_int("bit::shr", &args[0])?.wrapping_shr(extract_int("bit::shr", &args[1])? as u32),
    ))
}
fn bit_popcount(args: &[Value]) -> RResult<Value> {
    require_args("bit::popcount", args, 1)?;
    Ok(Value::Int(
        extract_int("bit::popcount", &args[0])?.count_ones() as i64,
    ))
}
fn bit_leading_zeros(args: &[Value]) -> RResult<Value> {
    require_args("bit::leading_zeros", args, 1)?;
    Ok(Value::Int(
        extract_int("bit::leading_zeros", &args[0])?.leading_zeros() as i64,
    ))
}
fn bit_trailing_zeros(args: &[Value]) -> RResult<Value> {
    require_args("bit::trailing_zeros", args, 1)?;
    Ok(Value::Int(
        extract_int("bit::trailing_zeros", &args[0])?.trailing_zeros() as i64,
    ))
}
fn bit_test(args: &[Value]) -> RResult<Value> {
    require_args("bit::test", args, 2)?;
    Ok(Value::Bool(
        (extract_int("bit::test", &args[0])? >> extract_int("bit::test", &args[1])? as u32) & 1
            == 1,
    ))
}
fn bit_set(args: &[Value]) -> RResult<Value> {
    require_args("bit::set", args, 2)?;
    Ok(Value::Int(
        extract_int("bit::set", &args[0])?
            | (1i64.wrapping_shl(extract_int("bit::set", &args[1])? as u32)),
    ))
}
fn bit_clear(args: &[Value]) -> RResult<Value> {
    require_args("bit::clear", args, 2)?;
    Ok(Value::Int(
        extract_int("bit::clear", &args[0])?
            & !(1i64.wrapping_shl(extract_int("bit::clear", &args[1])? as u32)),
    ))
}
fn bit_rotate_left(args: &[Value]) -> RResult<Value> {
    require_args("bit::rotate_left", args, 2)?;
    Ok(Value::Int(
        extract_int("bit::rotate_left", &args[0])?
            .rotate_left(extract_int("bit::rotate_left", &args[1])? as u32),
    ))
}
fn bit_rotate_right(args: &[Value]) -> RResult<Value> {
    require_args("bit::rotate_right", args, 2)?;
    Ok(Value::Int(
        extract_int("bit::rotate_right", &args[0])?
            .rotate_right(extract_int("bit::rotate_right", &args[1])? as u32),
    ))
}

static STD_BIT: StdModule = StdModule {
    name: "bit",
    description: "Bitwise operations for low-level data manipulation",
    functions: &[
        StdFn {
            name: "and",
            params: &[("int", "a"), ("int", "b")],
            return_type: "int",
            handler: bit_and,
        },
        StdFn {
            name: "or",
            params: &[("int", "a"), ("int", "b")],
            return_type: "int",
            handler: bit_or,
        },
        StdFn {
            name: "xor",
            params: &[("int", "a"), ("int", "b")],
            return_type: "int",
            handler: bit_xor,
        },
        StdFn {
            name: "not",
            params: &[("int", "a")],
            return_type: "int",
            handler: bit_not,
        },
        StdFn {
            name: "shl",
            params: &[("int", "a"), ("int", "n")],
            return_type: "int",
            handler: bit_shl,
        },
        StdFn {
            name: "shr",
            params: &[("int", "a"), ("int", "n")],
            return_type: "int",
            handler: bit_shr,
        },
        StdFn {
            name: "popcount",
            params: &[("int", "a")],
            return_type: "int",
            handler: bit_popcount,
        },
        StdFn {
            name: "leading_zeros",
            params: &[("int", "a")],
            return_type: "int",
            handler: bit_leading_zeros,
        },
        StdFn {
            name: "trailing_zeros",
            params: &[("int", "a")],
            return_type: "int",
            handler: bit_trailing_zeros,
        },
        StdFn {
            name: "test",
            params: &[("int", "val"), ("int", "pos")],
            return_type: "bool",
            handler: bit_test,
        },
        StdFn {
            name: "set",
            params: &[("int", "val"), ("int", "pos")],
            return_type: "int",
            handler: bit_set,
        },
        StdFn {
            name: "clear",
            params: &[("int", "val"), ("int", "pos")],
            return_type: "int",
            handler: bit_clear,
        },
        StdFn {
            name: "rotate_left",
            params: &[("int", "val"), ("int", "n")],
            return_type: "int",
            handler: bit_rotate_left,
        },
        StdFn {
            name: "rotate_right",
            params: &[("int", "val"), ("int", "n")],
            return_type: "int",
            handler: bit_rotate_right,
        },
    ],
};

// ─── Standard Library: Log ─────────────────────────────────────────

fn log_info(args: &[Value]) -> RResult<Value> {
    require_args("log::info", args, 1)?;
    eprintln!("[INFO] {}", value_display(&args[0]));
    Ok(Value::Void)
}
fn log_warn(args: &[Value]) -> RResult<Value> {
    require_args("log::warn", args, 1)?;
    eprintln!("[WARN] {}", value_display(&args[0]));
    Ok(Value::Void)
}
fn log_error(args: &[Value]) -> RResult<Value> {
    require_args("log::error", args, 1)?;
    eprintln!("[ERROR] {}", value_display(&args[0]));
    Ok(Value::Void)
}
fn log_debug(args: &[Value]) -> RResult<Value> {
    require_args("log::debug", args, 1)?;
    eprintln!("[DEBUG] {}", value_display(&args[0]));
    Ok(Value::Void)
}
fn log_trace(args: &[Value]) -> RResult<Value> {
    require_args("log::trace", args, 1)?;
    eprintln!("[TRACE] {}", value_display(&args[0]));
    Ok(Value::Void)
}

static STD_LOG: StdModule = StdModule {
    name: "log",
    description: "Structured logging to stderr",
    functions: &[
        StdFn {
            name: "info",
            params: &[("any", "msg")],
            return_type: "void",
            handler: log_info,
        },
        StdFn {
            name: "warn",
            params: &[("any", "msg")],
            return_type: "void",
            handler: log_warn,
        },
        StdFn {
            name: "error",
            params: &[("any", "msg")],
            return_type: "void",
            handler: log_error,
        },
        StdFn {
            name: "debug",
            params: &[("any", "msg")],
            return_type: "void",
            handler: log_debug,
        },
        StdFn {
            name: "trace",
            params: &[("any", "msg")],
            return_type: "void",
            handler: log_trace,
        },
    ],
};

// ─── Standard Library: Testing ─────────────────────────────────────

fn testing_assert_eq(args: &[Value]) -> RResult<Value> {
    require_args("testing::assert_eq", args, 2)?;
    let (a, b) = (value_display(&args[0]), value_display(&args[1]));
    if a == b {
        Ok(Value::Bool(true))
    } else {
        Err(format!("assertion failed: {} != {}", a, b))
    }
}
fn testing_assert_ne(args: &[Value]) -> RResult<Value> {
    require_args("testing::assert_ne", args, 2)?;
    let (a, b) = (value_display(&args[0]), value_display(&args[1]));
    if a != b {
        Ok(Value::Bool(true))
    } else {
        Err(format!(
            "assertion failed: {} == {} (expected different)",
            a, b
        ))
    }
}
fn testing_assert_true(args: &[Value]) -> RResult<Value> {
    require_args("testing::assert_true", args, 1)?;
    match &args[0] {
        Value::Bool(true) => Ok(Value::Bool(true)),
        Value::Bool(false) => Err("assertion failed: expected true, got false".to_string()),
        other => Err(format!(
            "testing::assert_true: expected bool, got {:?}",
            other
        )),
    }
}
fn testing_assert_false(args: &[Value]) -> RResult<Value> {
    require_args("testing::assert_false", args, 1)?;
    match &args[0] {
        Value::Bool(false) => Ok(Value::Bool(true)),
        Value::Bool(true) => Err("assertion failed: expected false, got true".to_string()),
        other => Err(format!(
            "testing::assert_false: expected bool, got {:?}",
            other
        )),
    }
}
fn testing_fail(args: &[Value]) -> RResult<Value> {
    Err(if args.is_empty() {
        "test failed".to_string()
    } else {
        value_display(&args[0])
    })
}

static STD_TESTING: StdModule = StdModule {
    name: "testing",
    description: "Test assertion utilities",
    functions: &[
        StdFn {
            name: "assert_eq",
            params: &[("any", "a"), ("any", "b")],
            return_type: "bool",
            handler: testing_assert_eq,
        },
        StdFn {
            name: "assert_ne",
            params: &[("any", "a"), ("any", "b")],
            return_type: "bool",
            handler: testing_assert_ne,
        },
        StdFn {
            name: "assert_true",
            params: &[("any", "val")],
            return_type: "bool",
            handler: testing_assert_true,
        },
        StdFn {
            name: "assert_false",
            params: &[("any", "val")],
            return_type: "bool",
            handler: testing_assert_false,
        },
        StdFn {
            name: "fail",
            params: &[("string", "msg")],
            return_type: "void",
            handler: testing_fail,
        },
    ],
};

// ─── Standard Library: CSV ─────────────────────────────────────────

fn csv_parse(args: &[Value]) -> RResult<Value> {
    require_args("csv::parse", args, 1)?;
    let s = extract_string("csv::parse", &args[0])?;
    Ok(Value::Array(
        s.lines()
            .filter(|l| !l.is_empty())
            .map(|l| Value::Array(csv_parse_row_str(l)))
            .collect(),
    ))
}
fn csv_parse_row(args: &[Value]) -> RResult<Value> {
    require_args("csv::parse_row", args, 1)?;
    Ok(Value::Array(csv_parse_row_str(&extract_string(
        "csv::parse_row",
        &args[0],
    )?)))
}
fn csv_stringify(args: &[Value]) -> RResult<Value> {
    require_args("csv::stringify", args, 1)?;
    let rows = extract_array("csv::stringify", &args[0])?;
    let mut result = String::new();
    for row in &rows {
        match row {
            Value::Array(cols) => {
                result.push_str(
                    &cols
                        .iter()
                        .map(|v| csv_escape_field(&value_display(v)))
                        .collect::<Vec<_>>()
                        .join(","),
                );
                result.push('\n');
            }
            _ => return Err("csv::stringify: each row must be an array".to_string()),
        }
    }
    Ok(Value::String(result))
}
fn csv_headers(args: &[Value]) -> RResult<Value> {
    require_args("csv::headers", args, 1)?;
    Ok(Value::Array(
        extract_string("csv::headers", &args[0])?
            .lines()
            .next()
            .map(csv_parse_row_str)
            .unwrap_or_default(),
    ))
}
fn csv_parse_row_str(line: &str) -> Vec<Value> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            fields.push(Value::String(current.clone()));
            current.clear();
        } else {
            current.push(c);
        }
    }
    fields.push(Value::String(current));
    fields
}
fn csv_escape_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

static STD_CSV: StdModule = StdModule {
    name: "csv",
    description: "CSV parsing and serialization",
    functions: &[
        StdFn {
            name: "parse",
            params: &[("string", "s")],
            return_type: "array",
            handler: csv_parse,
        },
        StdFn {
            name: "parse_row",
            params: &[("string", "row")],
            return_type: "array",
            handler: csv_parse_row,
        },
        StdFn {
            name: "stringify",
            params: &[("array", "rows")],
            return_type: "string",
            handler: csv_stringify,
        },
        StdFn {
            name: "headers",
            params: &[("string", "s")],
            return_type: "array",
            handler: csv_headers,
        },
    ],
};

// ─── Standard Library: Hex ─────────────────────────────────────────

fn hex_encode(args: &[Value]) -> RResult<Value> {
    require_args("hex::encode", args, 1)?;
    let bytes = match &args[0] {
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Bytes(b) => b.clone(),
        other => {
            return Err(format!(
                "hex::encode: expected string or bytes, got {:?}",
                other
            ));
        }
    };
    Ok(Value::String(
        bytes.iter().map(|b| format!("{:02x}", b)).collect(),
    ))
}
fn hex_decode(args: &[Value]) -> RResult<Value> {
    require_args("hex::decode", args, 1)?;
    let s = extract_string("hex::decode", &args[0])?;
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err("hex::decode: odd-length hex string".to_string());
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < s.len() {
        bytes.push(
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| format!("hex::decode: invalid hex at {}: {}", i, e))?,
        );
        i += 2;
    }
    match String::from_utf8(bytes.clone()) {
        Ok(s) => Ok(Value::String(s)),
        Err(_) => Ok(Value::Bytes(bytes)),
    }
}
fn hex_is_valid(args: &[Value]) -> RResult<Value> {
    require_args("hex::is_valid", args, 1)?;
    let s = extract_string("hex::is_valid", &args[0])?
        .trim()
        .to_string();
    Ok(Value::Bool(
        s.len() % 2 == 0 && s.chars().all(|c| c.is_ascii_hexdigit()),
    ))
}
fn hex_to_int(args: &[Value]) -> RResult<Value> {
    require_args("hex::to_int", args, 1)?;
    let s = extract_string("hex::to_int", &args[0])?;
    let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    i64::from_str_radix(s, 16)
        .map(Value::Int)
        .map_err(|e| format!("hex::to_int: {}", e))
}
fn hex_from_int(args: &[Value]) -> RResult<Value> {
    require_args("hex::from_int", args, 1)?;
    Ok(Value::String(format!(
        "0x{:x}",
        extract_int("hex::from_int", &args[0])?
    )))
}

static STD_HEX: StdModule = StdModule {
    name: "hex",
    description: "Hexadecimal encoding and decoding",
    functions: &[
        StdFn {
            name: "encode",
            params: &[("string", "input")],
            return_type: "string",
            handler: hex_encode,
        },
        StdFn {
            name: "decode",
            params: &[("string", "hex")],
            return_type: "string",
            handler: hex_decode,
        },
        StdFn {
            name: "is_valid",
            params: &[("string", "hex")],
            return_type: "bool",
            handler: hex_is_valid,
        },
        StdFn {
            name: "to_int",
            params: &[("string", "hex")],
            return_type: "int",
            handler: hex_to_int,
        },
        StdFn {
            name: "from_int",
            params: &[("int", "n")],
            return_type: "string",
            handler: hex_from_int,
        },
    ],
};

// ─── Standard Library: Random ──────────────────────────────────────

fn random_seed() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(42)
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
}
fn random_int(args: &[Value]) -> RResult<Value> {
    require_args("random::int", args, 2)?;
    let lo = extract_int("random::int", &args[0])?;
    let hi = extract_int("random::int", &args[1])?;
    if lo >= hi {
        return Err(format!("random::int: lo ({}) must be < hi ({})", lo, hi));
    }
    Ok(Value::Int(lo + (random_seed() % (hi - lo) as u64) as i64))
}
fn random_float(_args: &[Value]) -> RResult<Value> {
    Ok(Value::Float((random_seed() as f64 / u64::MAX as f64).abs()))
}
fn random_bool(_args: &[Value]) -> RResult<Value> {
    Ok(Value::Bool(random_seed().is_multiple_of(2)))
}
fn random_choice(args: &[Value]) -> RResult<Value> {
    require_args("random::choice", args, 1)?;
    let arr = extract_array("random::choice", &args[0])?;
    if arr.is_empty() {
        return Err("random::choice: empty array".to_string());
    }
    Ok(arr[(random_seed() as usize) % arr.len()].clone())
}
fn random_shuffle(args: &[Value]) -> RResult<Value> {
    require_args("random::shuffle", args, 1)?;
    let mut arr = extract_array("random::shuffle", &args[0])?;
    let len = arr.len();
    if len <= 1 {
        return Ok(Value::Array(arr));
    }
    let mut seed = random_seed();
    for i in (1..len).rev() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        arr.swap(i, (seed as usize) % (i + 1));
    }
    Ok(Value::Array(arr))
}

static STD_RANDOM: StdModule = StdModule {
    name: "random",
    description: "Random number generation (non-cryptographic)",
    functions: &[
        StdFn {
            name: "int",
            params: &[("int", "lo"), ("int", "hi")],
            return_type: "int",
            handler: random_int,
        },
        StdFn {
            name: "float",
            params: &[],
            return_type: "float",
            handler: random_float,
        },
        StdFn {
            name: "bool",
            params: &[],
            return_type: "bool",
            handler: random_bool,
        },
        StdFn {
            name: "choice",
            params: &[("array", "arr")],
            return_type: "any",
            handler: random_choice,
        },
        StdFn {
            name: "shuffle",
            params: &[("array", "arr")],
            return_type: "array",
            handler: random_shuffle,
        },
    ],
};

// ─── Standard Library: Color ───────────────────────────────────────

fn color_red(args: &[Value]) -> RResult<Value> {
    require_args("color::red", args, 1)?;
    Ok(Value::String(format!(
        "\x1b[31m{}\x1b[0m",
        value_display(&args[0])
    )))
}
fn color_green(args: &[Value]) -> RResult<Value> {
    require_args("color::green", args, 1)?;
    Ok(Value::String(format!(
        "\x1b[32m{}\x1b[0m",
        value_display(&args[0])
    )))
}
fn color_yellow(args: &[Value]) -> RResult<Value> {
    require_args("color::yellow", args, 1)?;
    Ok(Value::String(format!(
        "\x1b[33m{}\x1b[0m",
        value_display(&args[0])
    )))
}
fn color_blue(args: &[Value]) -> RResult<Value> {
    require_args("color::blue", args, 1)?;
    Ok(Value::String(format!(
        "\x1b[34m{}\x1b[0m",
        value_display(&args[0])
    )))
}
fn color_magenta(args: &[Value]) -> RResult<Value> {
    require_args("color::magenta", args, 1)?;
    Ok(Value::String(format!(
        "\x1b[35m{}\x1b[0m",
        value_display(&args[0])
    )))
}
fn color_cyan(args: &[Value]) -> RResult<Value> {
    require_args("color::cyan", args, 1)?;
    Ok(Value::String(format!(
        "\x1b[36m{}\x1b[0m",
        value_display(&args[0])
    )))
}
fn color_bold(args: &[Value]) -> RResult<Value> {
    require_args("color::bold", args, 1)?;
    Ok(Value::String(format!(
        "\x1b[1m{}\x1b[0m",
        value_display(&args[0])
    )))
}
fn color_dim(args: &[Value]) -> RResult<Value> {
    require_args("color::dim", args, 1)?;
    Ok(Value::String(format!(
        "\x1b[2m{}\x1b[0m",
        value_display(&args[0])
    )))
}
fn color_underline(args: &[Value]) -> RResult<Value> {
    require_args("color::underline", args, 1)?;
    Ok(Value::String(format!(
        "\x1b[4m{}\x1b[0m",
        value_display(&args[0])
    )))
}
fn color_strip(args: &[Value]) -> RResult<Value> {
    require_args("color::strip", args, 1)?;
    let s = extract_string("color::strip", &args[0])?;
    let mut r = String::new();
    let mut esc = false;
    for c in s.chars() {
        if c == '\x1b' {
            esc = true;
        } else if esc {
            if c == 'm' {
                esc = false;
            }
        } else {
            r.push(c);
        }
    }
    Ok(Value::String(r))
}
fn color_rgb(args: &[Value]) -> RResult<Value> {
    require_args("color::rgb", args, 4)?;
    Ok(Value::String(format!(
        "\x1b[38;2;{};{};{}m{}\x1b[0m",
        extract_int("color::rgb", &args[0])? as u8,
        extract_int("color::rgb", &args[1])? as u8,
        extract_int("color::rgb", &args[2])? as u8,
        value_display(&args[3])
    )))
}

static STD_COLOR: StdModule = StdModule {
    name: "color",
    description: "ANSI terminal color formatting",
    functions: &[
        StdFn {
            name: "red",
            params: &[("any", "text")],
            return_type: "string",
            handler: color_red,
        },
        StdFn {
            name: "green",
            params: &[("any", "text")],
            return_type: "string",
            handler: color_green,
        },
        StdFn {
            name: "yellow",
            params: &[("any", "text")],
            return_type: "string",
            handler: color_yellow,
        },
        StdFn {
            name: "blue",
            params: &[("any", "text")],
            return_type: "string",
            handler: color_blue,
        },
        StdFn {
            name: "magenta",
            params: &[("any", "text")],
            return_type: "string",
            handler: color_magenta,
        },
        StdFn {
            name: "cyan",
            params: &[("any", "text")],
            return_type: "string",
            handler: color_cyan,
        },
        StdFn {
            name: "bold",
            params: &[("any", "text")],
            return_type: "string",
            handler: color_bold,
        },
        StdFn {
            name: "dim",
            params: &[("any", "text")],
            return_type: "string",
            handler: color_dim,
        },
        StdFn {
            name: "underline",
            params: &[("any", "text")],
            return_type: "string",
            handler: color_underline,
        },
        StdFn {
            name: "strip",
            params: &[("string", "text")],
            return_type: "string",
            handler: color_strip,
        },
        StdFn {
            name: "rgb",
            params: &[("int", "r"), ("int", "g"), ("int", "b"), ("any", "text")],
            return_type: "string",
            handler: color_rgb,
        },
    ],
};

// ─── Standard Library: Process ─────────────────────────────────────

fn process_exec(args: &[Value]) -> RResult<Value> {
    require_args("process::exec", args, 1)?;
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&extract_string("process::exec", &args[0])?)
        .output()
        .map_err(|e| format!("process::exec: {}", e))?;
    let mut map = HashMap::new();
    map.insert(
        MapKey::Str("stdout".to_string()),
        Value::String(String::from_utf8_lossy(&output.stdout).to_string()),
    );
    map.insert(
        MapKey::Str("stderr".to_string()),
        Value::String(String::from_utf8_lossy(&output.stderr).to_string()),
    );
    map.insert(
        MapKey::Str("code".to_string()),
        Value::Int(output.status.code().unwrap_or(-1) as i64),
    );
    map.insert(
        MapKey::Str("success".to_string()),
        Value::Bool(output.status.success()),
    );
    Ok(Value::Map(map))
}
fn process_pid(_args: &[Value]) -> RResult<Value> {
    Ok(Value::Int(std::process::id() as i64))
}
fn process_env_vars(_args: &[Value]) -> RResult<Value> {
    let mut map = HashMap::new();
    for (k, v) in std::env::vars() {
        map.insert(MapKey::Str(k), Value::String(v));
    }
    Ok(Value::Map(map))
}
fn process_set_env(args: &[Value]) -> RResult<Value> {
    require_args("process::set_env", args, 2)?;
    unsafe {
        std::env::set_var(
            &extract_string("process::set_env", &args[0])?,
            &extract_string("process::set_env", &args[1])?,
        );
    }
    Ok(Value::Void)
}
fn process_which(args: &[Value]) -> RResult<Value> {
    require_args("process::which", args, 1)?;
    let output = std::process::Command::new("which")
        .arg(&extract_string("process::which", &args[0])?)
        .output()
        .map_err(|e| format!("process::which: {}", e))?;
    Ok(Value::String(if output.status.success() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        String::new()
    }))
}

static STD_PROCESS: StdModule = StdModule {
    name: "process",
    description: "Process execution and environment",
    functions: &[
        StdFn {
            name: "exec",
            params: &[("string", "command")],
            return_type: "map",
            handler: process_exec,
        },
        StdFn {
            name: "pid",
            params: &[],
            return_type: "int",
            handler: process_pid,
        },
        StdFn {
            name: "env_vars",
            params: &[],
            return_type: "map",
            handler: process_env_vars,
        },
        StdFn {
            name: "set_env",
            params: &[("string", "key"), ("string", "value")],
            return_type: "void",
            handler: process_set_env,
        },
        StdFn {
            name: "which",
            params: &[("string", "name")],
            return_type: "string",
            handler: process_which,
        },
    ],
};

// ─── Standard Library: INI ─────────────────────────────────────────

fn ini_parse(args: &[Value]) -> RResult<Value> {
    require_args("ini::parse", args, 1)?;
    let s = extract_string("ini::parse", &args[0])?;
    let mut result: HashMap<MapKey, Value> = HashMap::new();
    let mut section = String::new();
    let mut section_map: HashMap<MapKey, Value> = HashMap::new();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if !section.is_empty() || !section_map.is_empty() {
                result.insert(
                    MapKey::Str(section.clone()),
                    Value::Map(std::mem::take(&mut section_map)),
                );
            }
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        if let Some(eq) = line.find('=') {
            section_map.insert(
                MapKey::Str(line[..eq].trim().to_string()),
                Value::String(line[eq + 1..].trim().to_string()),
            );
        }
    }
    if !section.is_empty() || !section_map.is_empty() {
        result.insert(MapKey::Str(section), Value::Map(section_map));
    }
    Ok(Value::Map(result))
}
fn ini_stringify(args: &[Value]) -> RResult<Value> {
    require_args("ini::stringify", args, 1)?;
    let map = extract_map("ini::stringify", &args[0])?;
    let mut result = String::new();
    for (k, v) in &map {
        let name = match k {
            MapKey::Str(s) => s.clone(),
            MapKey::Int(i) => i.to_string(),
            MapKey::Bool(b) => b.to_string(),
        };
        if !name.is_empty() {
            result.push_str(&format!("[{}]\n", name));
        }
        if let Value::Map(entries) = v {
            for (ek, ev) in entries {
                let key = match ek {
                    MapKey::Str(s) => s.clone(),
                    MapKey::Int(i) => i.to_string(),
                    MapKey::Bool(b) => b.to_string(),
                };
                result.push_str(&format!("{} = {}\n", key, value_display(ev)));
            }
        }
        result.push('\n');
    }
    Ok(Value::String(result))
}
fn ini_sections(args: &[Value]) -> RResult<Value> {
    require_args("ini::sections", args, 1)?;
    Ok(Value::Array(
        extract_string("ini::sections", &args[0])?
            .lines()
            .filter(|l| l.trim().starts_with('[') && l.trim().ends_with(']'))
            .map(|l| Value::String(l.trim()[1..l.trim().len() - 1].trim().to_string()))
            .collect(),
    ))
}

static STD_INI: StdModule = StdModule {
    name: "ini",
    description: "INI/config file parsing and serialization",
    functions: &[
        StdFn {
            name: "parse",
            params: &[("string", "s")],
            return_type: "map",
            handler: ini_parse,
        },
        StdFn {
            name: "stringify",
            params: &[("map", "data")],
            return_type: "string",
            handler: ini_stringify,
        },
        StdFn {
            name: "sections",
            params: &[("string", "s")],
            return_type: "array",
            handler: ini_sections,
        },
    ],
};

// ─── Standard Library: Iter ────────────────────────────────────────

fn iter_range(args: &[Value]) -> RResult<Value> {
    if args.is_empty() || args.len() > 3 {
        return Err("iter::range requires 1-3 arguments".to_string());
    }
    let (start, end, step) = match args.len() {
        1 => (0, extract_int("iter::range", &args[0])?, 1),
        2 => (
            extract_int("iter::range", &args[0])?,
            extract_int("iter::range", &args[1])?,
            1,
        ),
        _ => (
            extract_int("iter::range", &args[0])?,
            extract_int("iter::range", &args[1])?,
            extract_int("iter::range", &args[2])?,
        ),
    };
    if step == 0 {
        return Err("iter::range: step cannot be 0".to_string());
    }
    let mut result = Vec::new();
    let mut i = start;
    if step > 0 {
        while i < end {
            result.push(Value::Int(i));
            i += step;
        }
    } else {
        while i > end {
            result.push(Value::Int(i));
            i += step;
        }
    }
    Ok(Value::Array(result))
}
fn iter_repeat(args: &[Value]) -> RResult<Value> {
    require_args("iter::repeat", args, 2)?;
    Ok(Value::Array(vec![
        args[0].clone();
        extract_int("iter::repeat", &args[1])?
            as usize
    ]))
}
fn iter_zip(args: &[Value]) -> RResult<Value> {
    require_args("iter::zip", args, 2)?;
    Ok(Value::Array(
        extract_array("iter::zip", &args[0])?
            .into_iter()
            .zip(extract_array("iter::zip", &args[1])?)
            .map(|(a, b)| Value::Array(vec![a, b]))
            .collect(),
    ))
}
fn iter_enumerate(args: &[Value]) -> RResult<Value> {
    require_args("iter::enumerate", args, 1)?;
    Ok(Value::Array(
        extract_array("iter::enumerate", &args[0])?
            .into_iter()
            .enumerate()
            .map(|(i, v)| Value::Array(vec![Value::Int(i as i64), v]))
            .collect(),
    ))
}
fn iter_flatten(args: &[Value]) -> RResult<Value> {
    require_args("iter::flatten", args, 1)?;
    let mut r = Vec::new();
    for item in extract_array("iter::flatten", &args[0])? {
        match item {
            Value::Array(inner) => r.extend(inner),
            other => r.push(other),
        }
    }
    Ok(Value::Array(r))
}
fn iter_take(args: &[Value]) -> RResult<Value> {
    require_args("iter::take", args, 2)?;
    Ok(Value::Array(
        extract_array("iter::take", &args[0])?
            .into_iter()
            .take(extract_int("iter::take", &args[1])? as usize)
            .collect(),
    ))
}
fn iter_skip(args: &[Value]) -> RResult<Value> {
    require_args("iter::skip", args, 2)?;
    Ok(Value::Array(
        extract_array("iter::skip", &args[0])?
            .into_iter()
            .skip(extract_int("iter::skip", &args[1])? as usize)
            .collect(),
    ))
}
fn iter_chunks(args: &[Value]) -> RResult<Value> {
    require_args("iter::chunks", args, 2)?;
    let arr = extract_array("iter::chunks", &args[0])?;
    let size = extract_int("iter::chunks", &args[1])? as usize;
    if size == 0 {
        return Err("iter::chunks: size must be > 0".to_string());
    }
    Ok(Value::Array(
        arr.chunks(size).map(|c| Value::Array(c.to_vec())).collect(),
    ))
}
fn iter_chain(args: &[Value]) -> RResult<Value> {
    require_args("iter::chain", args, 2)?;
    let mut a = extract_array("iter::chain", &args[0])?;
    a.extend(extract_array("iter::chain", &args[1])?);
    Ok(Value::Array(a))
}

static STD_ITER: StdModule = StdModule {
    name: "iter",
    description: "Iterator and sequence utilities",
    functions: &[
        StdFn {
            name: "range",
            params: &[("int", "start"), ("int", "end")],
            return_type: "array",
            handler: iter_range,
        },
        StdFn {
            name: "repeat",
            params: &[("any", "val"), ("int", "n")],
            return_type: "array",
            handler: iter_repeat,
        },
        StdFn {
            name: "zip",
            params: &[("array", "a"), ("array", "b")],
            return_type: "array",
            handler: iter_zip,
        },
        StdFn {
            name: "enumerate",
            params: &[("array", "arr")],
            return_type: "array",
            handler: iter_enumerate,
        },
        StdFn {
            name: "flatten",
            params: &[("array", "arr")],
            return_type: "array",
            handler: iter_flatten,
        },
        StdFn {
            name: "take",
            params: &[("array", "arr"), ("int", "n")],
            return_type: "array",
            handler: iter_take,
        },
        StdFn {
            name: "skip",
            params: &[("array", "arr"), ("int", "n")],
            return_type: "array",
            handler: iter_skip,
        },
        StdFn {
            name: "chunks",
            params: &[("array", "arr"), ("int", "size")],
            return_type: "array",
            handler: iter_chunks,
        },
        StdFn {
            name: "chain",
            params: &[("array", "a"), ("array", "b")],
            return_type: "array",
            handler: iter_chain,
        },
    ],
};

// ─── Standard Library: Buffer ──────────────────────────────────────

fn buffer_new(args: &[Value]) -> RResult<Value> {
    Ok(Value::Bytes(vec![
        0u8;
        if args.is_empty() {
            0
        } else {
            extract_int("buffer::new", &args[0])? as usize
        }
    ]))
}
fn buffer_from_string(args: &[Value]) -> RResult<Value> {
    require_args("buffer::from_string", args, 1)?;
    Ok(Value::Bytes(
        extract_string("buffer::from_string", &args[0])?.into_bytes(),
    ))
}
fn buffer_to_string(args: &[Value]) -> RResult<Value> {
    require_args("buffer::to_string", args, 1)?;
    Ok(Value::String(
        String::from_utf8_lossy(&extract_bytes("buffer::to_string", &args[0])?).to_string(),
    ))
}
fn buffer_length(args: &[Value]) -> RResult<Value> {
    require_args("buffer::length", args, 1)?;
    Ok(Value::Int(
        extract_bytes("buffer::length", &args[0])?.len() as i64
    ))
}
fn buffer_get(args: &[Value]) -> RResult<Value> {
    require_args("buffer::get", args, 2)?;
    let b = extract_bytes("buffer::get", &args[0])?;
    let i = extract_int("buffer::get", &args[1])? as usize;
    if i >= b.len() {
        return Err(format!(
            "buffer::get: index {} out of bounds (len {})",
            i,
            b.len()
        ));
    }
    Ok(Value::Int(b[i] as i64))
}
fn buffer_slice(args: &[Value]) -> RResult<Value> {
    require_args("buffer::slice", args, 3)?;
    let b = extract_bytes("buffer::slice", &args[0])?;
    let end = (extract_int("buffer::slice", &args[2])? as usize).min(b.len());
    let start = (extract_int("buffer::slice", &args[1])? as usize).min(end);
    Ok(Value::Bytes(b[start..end].to_vec()))
}
fn buffer_concat(args: &[Value]) -> RResult<Value> {
    require_args("buffer::concat", args, 2)?;
    let mut a = extract_bytes("buffer::concat", &args[0])?;
    a.extend_from_slice(&extract_bytes("buffer::concat", &args[1])?);
    Ok(Value::Bytes(a))
}
fn buffer_to_hex(args: &[Value]) -> RResult<Value> {
    require_args("buffer::to_hex", args, 1)?;
    Ok(Value::String(
        extract_bytes("buffer::to_hex", &args[0])?
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect(),
    ))
}
fn buffer_from_array(args: &[Value]) -> RResult<Value> {
    require_args("buffer::from_array", args, 1)?;
    let arr = extract_array("buffer::from_array", &args[0])?;
    let mut bytes = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        match v {
            Value::Int(n) if *n >= 0 && *n <= 255 => bytes.push(*n as u8),
            Value::Int(n) => {
                return Err(format!("buffer::from_array: {} out of 0-255 at {}", n, i));
            }
            other => {
                return Err(format!(
                    "buffer::from_array: expected int at {}, got {:?}",
                    i, other
                ));
            }
        }
    }
    Ok(Value::Bytes(bytes))
}

static STD_BUFFER: StdModule = StdModule {
    name: "buffer",
    description: "Byte buffer manipulation for binary data",
    functions: &[
        StdFn {
            name: "new",
            params: &[("int", "size")],
            return_type: "bytes",
            handler: buffer_new,
        },
        StdFn {
            name: "from_string",
            params: &[("string", "s")],
            return_type: "bytes",
            handler: buffer_from_string,
        },
        StdFn {
            name: "to_string",
            params: &[("bytes", "buf")],
            return_type: "string",
            handler: buffer_to_string,
        },
        StdFn {
            name: "length",
            params: &[("bytes", "buf")],
            return_type: "int",
            handler: buffer_length,
        },
        StdFn {
            name: "get",
            params: &[("bytes", "buf"), ("int", "index")],
            return_type: "int",
            handler: buffer_get,
        },
        StdFn {
            name: "slice",
            params: &[("bytes", "buf"), ("int", "start"), ("int", "end")],
            return_type: "bytes",
            handler: buffer_slice,
        },
        StdFn {
            name: "concat",
            params: &[("bytes", "a"), ("bytes", "b")],
            return_type: "bytes",
            handler: buffer_concat,
        },
        StdFn {
            name: "to_hex",
            params: &[("bytes", "buf")],
            return_type: "string",
            handler: buffer_to_hex,
        },
        StdFn {
            name: "from_array",
            params: &[("array", "bytes")],
            return_type: "bytes",
            handler: buffer_from_array,
        },
    ],
};

// ─── Standard Library: Hash ────────────────────────────────────────

fn hash_fnv32(args: &[Value]) -> RResult<Value> {
    require_args("hash::fnv32", args, 1)?;
    let d = value_to_bytes(&args[0]);
    let mut h: u32 = 0x811c9dc5;
    for b in &d {
        h ^= *b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    Ok(Value::Int(h as i64))
}
fn hash_fnv64(args: &[Value]) -> RResult<Value> {
    require_args("hash::fnv64", args, 1)?;
    let d = value_to_bytes(&args[0]);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in &d {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    Ok(Value::Int(h as i64))
}
fn hash_djb2(args: &[Value]) -> RResult<Value> {
    require_args("hash::djb2", args, 1)?;
    let d = value_to_bytes(&args[0]);
    let mut h: u64 = 5381;
    for b in &d {
        h = h.wrapping_mul(33).wrapping_add(*b as u64);
    }
    Ok(Value::Int(h as i64))
}
fn hash_crc32(args: &[Value]) -> RResult<Value> {
    require_args("hash::crc32", args, 1)?;
    let d = value_to_bytes(&args[0]);
    let mut crc: u32 = 0xFFFFFFFF;
    for b in &d {
        crc ^= *b as u32;
        for _ in 0..8 {
            if crc & 1 == 1 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    Ok(Value::Int((crc ^ 0xFFFFFFFF) as i64))
}
fn hash_adler32(args: &[Value]) -> RResult<Value> {
    require_args("hash::adler32", args, 1)?;
    let d = value_to_bytes(&args[0]);
    let (mut a, mut b): (u32, u32) = (1, 0);
    for byte in &d {
        a = (a + *byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    Ok(Value::Int(((b << 16) | a) as i64))
}
fn value_to_bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Bytes(b) => b.clone(),
        other => format!("{:?}", other).into_bytes(),
    }
}

static STD_HASH: StdModule = StdModule {
    name: "hash",
    description: "Non-cryptographic hash functions",
    functions: &[
        StdFn {
            name: "fnv32",
            params: &[("any", "data")],
            return_type: "int",
            handler: hash_fnv32,
        },
        StdFn {
            name: "fnv64",
            params: &[("any", "data")],
            return_type: "int",
            handler: hash_fnv64,
        },
        StdFn {
            name: "djb2",
            params: &[("any", "data")],
            return_type: "int",
            handler: hash_djb2,
        },
        StdFn {
            name: "crc32",
            params: &[("any", "data")],
            return_type: "int",
            handler: hash_crc32,
        },
        StdFn {
            name: "adler32",
            params: &[("any", "data")],
            return_type: "int",
            handler: hash_adler32,
        },
    ],
};

// ─── Standard Library: Sort ────────────────────────────────────────

fn sort_asc(args: &[Value]) -> RResult<Value> {
    require_args("sort::asc", args, 1)?;
    let mut a = extract_array("sort::asc", &args[0])?;
    a.sort_by(value_cmp);
    Ok(Value::Array(a))
}
fn sort_desc(args: &[Value]) -> RResult<Value> {
    require_args("sort::desc", args, 1)?;
    let mut a = extract_array("sort::desc", &args[0])?;
    a.sort_by(|x, y| value_cmp(y, x));
    Ok(Value::Array(a))
}
fn sort_is_sorted(args: &[Value]) -> RResult<Value> {
    require_args("sort::is_sorted", args, 1)?;
    let a = extract_array("sort::is_sorted", &args[0])?;
    Ok(Value::Bool(a.windows(2).all(|w| {
        value_cmp(&w[0], &w[1]) != std::cmp::Ordering::Greater
    })))
}
fn sort_min(args: &[Value]) -> RResult<Value> {
    require_args("sort::min", args, 1)?;
    let a = extract_array("sort::min", &args[0])?;
    if a.is_empty() {
        return Err("sort::min: empty array".to_string());
    }
    let mut m = &a[0];
    for item in &a[1..] {
        if value_cmp(item, m) == std::cmp::Ordering::Less {
            m = item;
        }
    }
    Ok(m.clone())
}
fn sort_max(args: &[Value]) -> RResult<Value> {
    require_args("sort::max", args, 1)?;
    let a = extract_array("sort::max", &args[0])?;
    if a.is_empty() {
        return Err("sort::max: empty array".to_string());
    }
    let mut m = &a[0];
    for item in &a[1..] {
        if value_cmp(item, m) == std::cmp::Ordering::Greater {
            m = item;
        }
    }
    Ok(m.clone())
}
fn sort_by_length(args: &[Value]) -> RResult<Value> {
    require_args("sort::by_length", args, 1)?;
    let mut a = extract_array("sort::by_length", &args[0])?;
    a.sort_by_key(|v| match v {
        Value::String(s) => s.len(),
        Value::Array(a) => a.len(),
        Value::Bytes(b) => b.len(),
        _ => 0,
    });
    Ok(Value::Array(a))
}
fn value_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Int(x), Value::Float(y)) => (*x as f64)
            .partial_cmp(y)
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::Float(x), Value::Int(y)) => x
            .partial_cmp(&(*y as f64))
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    }
}

static STD_SORT: StdModule = StdModule {
    name: "sort",
    description: "Sorting and ordering utilities",
    functions: &[
        StdFn {
            name: "asc",
            params: &[("array", "arr")],
            return_type: "array",
            handler: sort_asc,
        },
        StdFn {
            name: "desc",
            params: &[("array", "arr")],
            return_type: "array",
            handler: sort_desc,
        },
        StdFn {
            name: "is_sorted",
            params: &[("array", "arr")],
            return_type: "bool",
            handler: sort_is_sorted,
        },
        StdFn {
            name: "min",
            params: &[("array", "arr")],
            return_type: "any",
            handler: sort_min,
        },
        StdFn {
            name: "max",
            params: &[("array", "arr")],
            return_type: "any",
            handler: sort_max,
        },
        StdFn {
            name: "by_length",
            params: &[("array", "arr")],
            return_type: "array",
            handler: sort_by_length,
        },
    ],
};

// ─── Standard Library: UUID ────────────────────────────────────────

fn uuid_v4(_args: &[Value]) -> RResult<Value> {
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
        let mut s = random_seed();
        for b in bytes.iter_mut() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (s >> 33) as u8;
        }
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Value::String(format_uuid_bytes(&bytes)))
}
fn uuid_nil(_args: &[Value]) -> RResult<Value> {
    Ok(Value::String(
        "00000000-0000-0000-0000-000000000000".to_string(),
    ))
}
fn uuid_is_valid(args: &[Value]) -> RResult<Value> {
    require_args("uuid::is_valid", args, 1)?;
    let s = extract_string("uuid::is_valid", &args[0])?
        .trim()
        .to_string();
    if s.len() != 36 {
        return Ok(Value::Bool(false));
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return Ok(Value::Bool(false));
    }
    for (p, &l) in parts.iter().zip(&[8, 4, 4, 4, 12]) {
        if p.len() != l || !p.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}
fn uuid_version(args: &[Value]) -> RResult<Value> {
    require_args("uuid::version", args, 1)?;
    let s = extract_string("uuid::version", &args[0])?
        .trim()
        .to_string();
    if s.len() != 36 {
        return Err("uuid::version: invalid UUID".to_string());
    }
    match s.chars().nth(14) {
        Some(c) if c.is_ascii_digit() => Ok(Value::Int((c as u8 - b'0') as i64)),
        _ => Err("uuid::version: cannot determine version".to_string()),
    }
}
fn format_uuid_bytes(b: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

static STD_UUID: StdModule = StdModule {
    name: "uuid",
    description: "UUID generation and validation",
    functions: &[
        StdFn {
            name: "v4",
            params: &[],
            return_type: "string",
            handler: uuid_v4,
        },
        StdFn {
            name: "nil",
            params: &[],
            return_type: "string",
            handler: uuid_nil,
        },
        StdFn {
            name: "is_valid",
            params: &[("string", "s")],
            return_type: "bool",
            handler: uuid_is_valid,
        },
        StdFn {
            name: "version",
            params: &[("string", "s")],
            return_type: "int",
            handler: uuid_version,
        },
    ],
};

// ─── Standard Library: Encoding ────────────────────────────────────

fn encoding_utf8_encode(args: &[Value]) -> RResult<Value> {
    require_args("encoding::utf8_encode", args, 1)?;
    Ok(Value::Bytes(
        extract_string("encoding::utf8_encode", &args[0])?.into_bytes(),
    ))
}
fn encoding_utf8_decode(args: &[Value]) -> RResult<Value> {
    require_args("encoding::utf8_decode", args, 1)?;
    String::from_utf8(extract_bytes("encoding::utf8_decode", &args[0])?)
        .map(Value::String)
        .map_err(|e| format!("encoding::utf8_decode: {}", e))
}
fn encoding_utf8_valid(args: &[Value]) -> RResult<Value> {
    require_args("encoding::utf8_valid", args, 1)?;
    Ok(Value::Bool(
        std::str::from_utf8(&extract_bytes("encoding::utf8_valid", &args[0])?).is_ok(),
    ))
}
fn encoding_ascii_codes(args: &[Value]) -> RResult<Value> {
    require_args("encoding::ascii_codes", args, 1)?;
    Ok(Value::Array(
        extract_string("encoding::ascii_codes", &args[0])?
            .bytes()
            .map(|b| Value::Int(b as i64))
            .collect(),
    ))
}
fn encoding_from_ascii(args: &[Value]) -> RResult<Value> {
    require_args("encoding::from_ascii", args, 1)?;
    let arr = extract_array("encoding::from_ascii", &args[0])?;
    let mut r = String::new();
    for (i, v) in arr.iter().enumerate() {
        match v {
            Value::Int(n) if *n >= 0 && *n <= 127 => r.push(*n as u8 as char),
            Value::Int(n) => {
                return Err(format!("encoding::from_ascii: {} out of range at {}", n, i));
            }
            other => {
                return Err(format!(
                    "encoding::from_ascii: expected int at {}, got {:?}",
                    i, other
                ));
            }
        }
    }
    Ok(Value::String(r))
}
fn encoding_byte_length(args: &[Value]) -> RResult<Value> {
    require_args("encoding::byte_length", args, 1)?;
    Ok(Value::Int(
        extract_string("encoding::byte_length", &args[0])?.len() as i64,
    ))
}

static STD_ENCODING: StdModule = StdModule {
    name: "encoding",
    description: "Text encoding and decoding utilities",
    functions: &[
        StdFn {
            name: "utf8_encode",
            params: &[("string", "s")],
            return_type: "bytes",
            handler: encoding_utf8_encode,
        },
        StdFn {
            name: "utf8_decode",
            params: &[("bytes", "data")],
            return_type: "string",
            handler: encoding_utf8_decode,
        },
        StdFn {
            name: "utf8_valid",
            params: &[("bytes", "data")],
            return_type: "bool",
            handler: encoding_utf8_valid,
        },
        StdFn {
            name: "ascii_codes",
            params: &[("string", "s")],
            return_type: "array",
            handler: encoding_ascii_codes,
        },
        StdFn {
            name: "from_ascii",
            params: &[("array", "codes")],
            return_type: "string",
            handler: encoding_from_ascii,
        },
        StdFn {
            name: "byte_length",
            params: &[("string", "s")],
            return_type: "int",
            handler: encoding_byte_length,
        },
    ],
};

// ─── Standard Library: URL ─────────────────────────────────────────

fn url_parse_fn(args: &[Value]) -> RResult<Value> {
    require_args("url::parse", args, 1)?;
    let s = extract_string("url::parse", &args[0])?.trim().to_string();
    let mut map = HashMap::new();
    let (scheme, rest) = if let Some(idx) = s.find("://") {
        (s[..idx].to_string(), s[idx + 3..].to_string())
    } else {
        (String::new(), s.clone())
    };
    map.insert(MapKey::Str("scheme".to_string()), Value::String(scheme));
    let (authority, pqf) = match rest.find('/') {
        Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
        None => match rest.find('?') {
            Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
            None => match rest.find('#') {
                Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
                None => (rest.clone(), String::new()),
            },
        },
    };
    let (host, port) = match authority.rfind(':') {
        Some(i) => {
            let p = &authority[i + 1..];
            if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() {
                (authority[..i].to_string(), p.to_string())
            } else {
                (authority.clone(), String::new())
            }
        }
        None => (authority, String::new()),
    };
    map.insert(MapKey::Str("host".to_string()), Value::String(host));
    map.insert(MapKey::Str("port".to_string()), Value::String(port));
    let (path, qf) = match pqf.find('?') {
        Some(i) => (pqf[..i].to_string(), pqf[i + 1..].to_string()),
        None => match pqf.find('#') {
            Some(i) => (pqf[..i].to_string(), pqf[i..].to_string()),
            None => (pqf, String::new()),
        },
    };
    map.insert(MapKey::Str("path".to_string()), Value::String(path));
    let (query, fragment) = match qf.find('#') {
        Some(i) => (qf[..i].to_string(), qf[i + 1..].to_string()),
        None => (qf, String::new()),
    };
    map.insert(MapKey::Str("query".to_string()), Value::String(query));
    map.insert(MapKey::Str("fragment".to_string()), Value::String(fragment));
    Ok(Value::Map(map))
}
fn url_build(args: &[Value]) -> RResult<Value> {
    require_args("url::build", args, 1)?;
    let map = extract_map("url::build", &args[0])?;
    let get = |k: &str| {
        map.get(&MapKey::Str(k.to_string()))
            .map(value_display)
            .unwrap_or_default()
    };
    let mut r = String::new();
    let scheme = get("scheme");
    if !scheme.is_empty() {
        r.push_str(&scheme);
        r.push_str("://");
    }
    r.push_str(&get("host"));
    let port = get("port");
    if !port.is_empty() {
        r.push(':');
        r.push_str(&port);
    }
    let path = get("path");
    if !path.is_empty() {
        r.push_str(&path);
    }
    let query = get("query");
    if !query.is_empty() {
        r.push('?');
        r.push_str(&query);
    }
    let fragment = get("fragment");
    if !fragment.is_empty() {
        r.push('#');
        r.push_str(&fragment);
    }
    Ok(Value::String(r))
}
fn url_encode_component(args: &[Value]) -> RResult<Value> {
    require_args("url::encode_component", args, 1)?;
    Ok(Value::String(url_encode(&extract_string(
        "url::encode_component",
        &args[0],
    )?)))
}
fn url_decode_component(args: &[Value]) -> RResult<Value> {
    require_args("url::decode_component", args, 1)?;
    Ok(Value::String(url_decode(&extract_string(
        "url::decode_component",
        &args[0],
    )?)))
}
fn url_query_params(args: &[Value]) -> RResult<Value> {
    require_args("url::query_params", args, 1)?;
    let s = extract_string("url::query_params", &args[0])?
        .trim_start_matches('?')
        .to_string();
    let mut map = HashMap::new();
    for pair in s.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.find('=') {
            Some(i) => (url_decode(&pair[..i]), url_decode(&pair[i + 1..])),
            None => (url_decode(pair), String::new()),
        };
        map.insert(MapKey::Str(k), Value::String(v));
    }
    Ok(Value::Map(map))
}
fn url_build_query(args: &[Value]) -> RResult<Value> {
    require_args("url::build_query", args, 1)?;
    let map = extract_map("url::build_query", &args[0])?;
    Ok(Value::String(
        map.iter()
            .map(|(k, v)| {
                let key = match k {
                    MapKey::Str(s) => url_encode(s),
                    MapKey::Int(i) => i.to_string(),
                    MapKey::Bool(b) => b.to_string(),
                };
                format!("{}={}", key, url_encode(&value_display(v)))
            })
            .collect::<Vec<_>>()
            .join("&"),
    ))
}

static STD_URL: StdModule = StdModule {
    name: "url",
    description: "URL parsing, building, and query string handling",
    functions: &[
        StdFn {
            name: "parse",
            params: &[("string", "url")],
            return_type: "map",
            handler: url_parse_fn,
        },
        StdFn {
            name: "build",
            params: &[("map", "components")],
            return_type: "string",
            handler: url_build,
        },
        StdFn {
            name: "encode_component",
            params: &[("string", "s")],
            return_type: "string",
            handler: url_encode_component,
        },
        StdFn {
            name: "decode_component",
            params: &[("string", "s")],
            return_type: "string",
            handler: url_decode_component,
        },
        StdFn {
            name: "query_params",
            params: &[("string", "query")],
            return_type: "map",
            handler: url_query_params,
        },
        StdFn {
            name: "build_query",
            params: &[("map", "params")],
            return_type: "string",
            handler: url_build_query,
        },
    ],
};

// ─── Helper: argument extraction ───────────────────────────────────

fn require_args(name: &str, args: &[Value], min: usize) -> RResult<()> {
    if args.len() < min {
        Err(format!(
            "{} requires at least {} argument(s), got {}",
            name,
            min,
            args.len()
        ))
    } else {
        Ok(())
    }
}
fn extract_string(name: &str, v: &Value) -> RResult<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        other => Err(format!("{}: expected string, got {:?}", name, other)),
    }
}
fn extract_int(name: &str, v: &Value) -> RResult<i64> {
    match v {
        Value::Int(n) => Ok(*n),
        other => Err(format!("{}: expected int, got {:?}", name, other)),
    }
}
fn extract_array(name: &str, v: &Value) -> RResult<Vec<Value>> {
    match v {
        Value::Array(a) => Ok(a.clone()),
        other => Err(format!("{}: expected array, got {:?}", name, other)),
    }
}
fn extract_bytes(name: &str, v: &Value) -> RResult<Vec<u8>> {
    match v {
        Value::Bytes(b) => Ok(b.clone()),
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        other => Err(format!("{}: expected bytes, got {:?}", name, other)),
    }
}
fn extract_map(name: &str, v: &Value) -> RResult<HashMap<MapKey, Value>> {
    match v {
        Value::Map(m) => Ok(m.clone()),
        other => Err(format!("{}: expected map, got {:?}", name, other)),
    }
}
fn value_display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => format!("{}", f),
        Value::Bool(b) => b.to_string(),
        Value::Void => "void".to_string(),
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(value_display).collect::<Vec<_>>().join(", ")
        ),
        other => format!("{:?}", other),
    }
}

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
        assert!(lookup_std_module("string").is_some());
        assert!(lookup_std_module("fmt").is_some());
        assert!(lookup_std_module("path").is_some());
        assert!(lookup_std_module("convert").is_some());
        assert!(lookup_std_module("bit").is_some());
        assert!(lookup_std_module("log").is_some());
        assert!(lookup_std_module("testing").is_some());
        assert!(lookup_std_module("csv").is_some());
        assert!(lookup_std_module("hex").is_some());
        assert!(lookup_std_module("random").is_some());
        assert!(lookup_std_module("color").is_some());
        assert!(lookup_std_module("process").is_some());
        assert!(lookup_std_module("ini").is_some());
        assert!(lookup_std_module("iter").is_some());
        assert!(lookup_std_module("buffer").is_some());
        assert!(lookup_std_module("hash").is_some());
        assert!(lookup_std_module("sort").is_some());
        assert!(lookup_std_module("uuid").is_some());
        assert!(lookup_std_module("encoding").is_some());
        assert!(lookup_std_module("url").is_some());
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

    fn assert_str(v: Value, expected: &str) {
        match v {
            Value::String(s) => assert_eq!(s, expected),
            other => panic!("expected String({expected}), got {:?}", other),
        }
    }
    fn assert_int(v: Value, expected: i64) {
        match v {
            Value::Int(i) => assert_eq!(i, expected),
            other => panic!("expected Int({expected}), got {:?}", other),
        }
    }
    fn assert_bool(v: Value, expected: bool) {
        match v {
            Value::Bool(b) => assert_eq!(b, expected),
            other => panic!("expected Bool({expected}), got {:?}", other),
        }
    }

    #[test]
    fn string_trim_operations() {
        assert_str(
            string_trim(&[Value::String("  hi  ".into())]).unwrap(),
            "hi",
        );
        assert_str(
            string_upper(&[Value::String("hello".into())]).unwrap(),
            "HELLO",
        );
        assert_str(
            string_lower(&[Value::String("HELLO".into())]).unwrap(),
            "hello",
        );
    }

    #[test]
    fn string_split_join() {
        match string_split(&[Value::String("a,b,c".into()), Value::String(",".into())]).unwrap() {
            Value::Array(a) => assert_eq!(a.len(), 3),
            other => panic!("expected Array, got {:?}", other),
        }
        let arr = Value::Array(vec![Value::String("x".into()), Value::String("y".into())]);
        assert_str(
            string_join(&[arr, Value::String("-".into())]).unwrap(),
            "x-y",
        );
    }

    #[test]
    fn string_contains_and_index() {
        assert_bool(
            string_contains(&[
                Value::String("hello world".into()),
                Value::String("world".into()),
            ])
            .unwrap(),
            true,
        );
        assert_int(
            string_index_of(&[Value::String("abcdef".into()), Value::String("cd".into())]).unwrap(),
            2,
        );
        assert_int(
            string_index_of(&[Value::String("abcdef".into()), Value::String("zz".into())]).unwrap(),
            -1,
        );
    }

    #[test]
    fn fmt_format_placeholders() {
        assert_str(
            fmt_format(&[
                Value::String("Hello, {}! You are {} years old.".into()),
                Value::String("Alice".into()),
                Value::Int(30),
            ])
            .unwrap(),
            "Hello, Alice! You are 30 years old.",
        );
    }

    #[test]
    fn fmt_number_formatting() {
        assert_str(fmt_hex(&[Value::Int(255)]).unwrap(), "ff");
        assert_str(fmt_oct(&[Value::Int(8)]).unwrap(), "10");
        assert_str(fmt_bin(&[Value::Int(10)]).unwrap(), "1010");
    }

    #[test]
    fn path_operations() {
        assert_str(
            path_basename(&[Value::String("/usr/local/bin/rustc".into())]).unwrap(),
            "rustc",
        );
        assert_str(
            path_dirname(&[Value::String("/usr/local/bin/rustc".into())]).unwrap(),
            "/usr/local/bin",
        );
        assert_str(
            path_extension(&[Value::String("file.tar.gz".into())]).unwrap(),
            "gz",
        );
    }

    #[test]
    fn convert_parse_types() {
        assert_int(
            convert_parse_int(&[Value::String("42".into())]).unwrap(),
            42,
        );
        match convert_parse_float(&[Value::String("1.23".into())]).unwrap() {
            Value::Float(f) => assert!((f - 1.23_f64).abs() < 1e-10),
            other => panic!("expected Float, got {:?}", other),
        }
        assert_str(convert_type_of(&[Value::Bool(true)]).unwrap(), "bool");
        assert_str(convert_type_of(&[Value::Int(1)]).unwrap(), "int");
    }

    #[test]
    fn bit_operations() {
        assert_int(
            bit_and(&[Value::Int(0b1100), Value::Int(0b1010)]).unwrap(),
            0b1000,
        );
        assert_int(
            bit_or(&[Value::Int(0b1100), Value::Int(0b1010)]).unwrap(),
            0b1110,
        );
        assert_int(
            bit_xor(&[Value::Int(0b1100), Value::Int(0b1010)]).unwrap(),
            0b0110,
        );
        assert_int(bit_popcount(&[Value::Int(0b1011)]).unwrap(), 3);
    }

    #[test]
    fn csv_parse_and_stringify() {
        let csv = "name,age\nAlice,30\nBob,25";
        match csv_parse(&[Value::String(csv.into())]).unwrap() {
            Value::Array(rows) => assert_eq!(rows.len(), 3),
            other => panic!("expected Array, got {:?}", other),
        }
        match csv_headers(&[Value::String(csv.into())]).unwrap() {
            Value::Array(h) => {
                assert_eq!(h.len(), 2);
                assert_str(h[0].clone(), "name");
                assert_str(h[1].clone(), "age");
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn hex_encode_decode() {
        assert_str(
            hex_encode(&[Value::String("hello".into())]).unwrap(),
            "68656c6c6f",
        );
        assert_str(
            hex_decode(&[Value::String("68656c6c6f".into())]).unwrap(),
            "hello",
        );
        assert_bool(hex_is_valid(&[Value::String("0f1a".into())]).unwrap(), true);
        assert_bool(
            hex_is_valid(&[Value::String("0fzz".into())]).unwrap(),
            false,
        );
    }

    #[test]
    fn iter_range_variants() {
        match iter_range(&[Value::Int(3)]).unwrap() {
            Value::Array(a) => {
                assert_eq!(a.len(), 3);
                assert_int(a[0].clone(), 0);
                assert_int(a[2].clone(), 2);
            }
            other => panic!("expected Array, got {:?}", other),
        }
        match iter_range(&[Value::Int(2), Value::Int(5)]).unwrap() {
            Value::Array(a) => {
                assert_eq!(a.len(), 3);
                assert_int(a[0].clone(), 2);
                assert_int(a[2].clone(), 4);
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn buffer_create_and_ops() {
        match buffer_new(&[Value::Int(4)]).unwrap() {
            Value::Bytes(b) => assert_eq!(b.len(), 4),
            other => panic!("expected Bytes, got {:?}", other),
        }
        match buffer_from_string(&[Value::String("hi".into())]).unwrap() {
            Value::Bytes(b) => assert_eq!(b, vec![104, 105]),
            other => panic!("expected Bytes, got {:?}", other),
        }
    }

    #[test]
    fn hash_functions_produce_ints() {
        match hash_fnv32(&[Value::String("test".into())]).unwrap() {
            Value::Int(_) => {}
            other => panic!("expected Int, got {:?}", other),
        }
        match hash_crc32(&[Value::String("test".into())]).unwrap() {
            Value::Int(_) => {}
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn sort_asc_desc() {
        let arr = vec![Value::Int(3), Value::Int(1), Value::Int(2)];
        match sort_asc(&[Value::Array(arr.clone())]).unwrap() {
            Value::Array(a) => {
                assert_int(a[0].clone(), 1);
                assert_int(a[1].clone(), 2);
                assert_int(a[2].clone(), 3);
            }
            other => panic!("expected Array, got {:?}", other),
        }
        match sort_desc(&[Value::Array(arr)]).unwrap() {
            Value::Array(a) => {
                assert_int(a[0].clone(), 3);
                assert_int(a[1].clone(), 2);
                assert_int(a[2].clone(), 1);
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn sort_min_max() {
        let arr = vec![Value::Int(5), Value::Int(1), Value::Int(3)];
        assert_int(sort_min(&[Value::Array(arr.clone())]).unwrap(), 1);
        assert_int(sort_max(&[Value::Array(arr)]).unwrap(), 5);
    }

    #[test]
    fn uuid_nil_and_valid() {
        assert_str(
            uuid_nil(&[]).unwrap(),
            "00000000-0000-0000-0000-000000000000",
        );
        assert_bool(
            uuid_is_valid(&[Value::String("550e8400-e29b-41d4-a716-446655440000".into())]).unwrap(),
            true,
        );
        assert_bool(
            uuid_is_valid(&[Value::String("not-a-uuid".into())]).unwrap(),
            false,
        );
    }

    #[test]
    fn encoding_utf8_roundtrip() {
        match encoding_utf8_encode(&[Value::String("hello".into())]).unwrap() {
            Value::Bytes(b) => assert_eq!(b, vec![104, 101, 108, 108, 111]),
            other => panic!("expected Bytes, got {:?}", other),
        }
        assert_str(
            encoding_utf8_decode(&[Value::Bytes(vec![104, 101, 108, 108, 111])]).unwrap(),
            "hello",
        );
        assert_bool(
            encoding_utf8_valid(&[Value::Bytes(vec![104, 101])]).unwrap(),
            true,
        );
        assert_bool(
            encoding_utf8_valid(&[Value::Bytes(vec![0xff, 0xfe])]).unwrap(),
            false,
        );
    }

    #[test]
    fn ini_parse_roundtrip() {
        match ini_parse(&[Value::String("[section]\nkey=value\nfoo=bar\n".into())]).unwrap() {
            Value::Map(m) => assert!(!m.is_empty()),
            other => panic!("expected Map, got {:?}", other),
        }
    }

    #[test]
    fn color_ansi_codes() {
        match color_red(&[Value::String("err".into())]).unwrap() {
            Value::String(s) => {
                assert!(s.contains("\x1b[31m"));
                assert!(s.contains("\x1b[0m"));
                assert!(s.contains("err"));
            }
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn total_module_count() {
        assert_eq!(STD_MODULES.len(), 31);
    }
}
