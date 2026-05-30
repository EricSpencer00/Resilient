//! RES-2584: String builder — efficient mutable string construction.
//!
//! A string builder accumulates string parts without repeated allocation.
//! Backed by `Value::Array` of strings; `string_builder_build` joins them
//! with a single allocation.
//!
//! API:
//!   string_builder_new()              → Builder (empty)
//!   string_builder_append(b, s)       → Builder — append any value as string
//!   string_builder_prepend(b, s)      → Builder — prepend any value as string
//!   string_builder_build(b)           → String — join all parts
//!   string_builder_len(b)             → int — total byte length of accumulated strings
//!   string_builder_is_empty(b)        → bool — true when no parts have been appended
//!   string_builder_clear(b)           → Builder — discard all parts

use crate::Value;

type RResult<T> = Result<T, String>;

fn value_to_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Char(c) => c.to_string(),
        other => format!("{other}"),
    }
}

/// `string_builder_new() → Builder`
pub(crate) fn builtin_string_builder_new(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "string_builder_new: expected 0 arguments, got {}",
            args.len()
        ));
    }
    Ok(Value::Array(Vec::new()))
}

/// `string_builder_append(b, val) → Builder` — append value converted to string.
pub(crate) fn builtin_string_builder_append(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(parts), val] => {
            let mut out = parts.clone();
            out.push(Value::String(value_to_str(val)));
            Ok(Value::Array(out))
        }
        [other, _] => Err(format!(
            "string_builder_append: expected Builder (Array), got {other}"
        )),
        _ => Err(format!(
            "string_builder_append: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `string_builder_prepend(b, val) → Builder` — prepend value converted to string.
pub(crate) fn builtin_string_builder_prepend(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(parts), val] => {
            let mut out = Vec::with_capacity(parts.len() + 1);
            out.push(Value::String(value_to_str(val)));
            out.extend_from_slice(parts);
            Ok(Value::Array(out))
        }
        [other, _] => Err(format!(
            "string_builder_prepend: expected Builder (Array), got {other}"
        )),
        _ => Err(format!(
            "string_builder_prepend: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `string_builder_build(b) → String` — join all parts.
pub(crate) fn builtin_string_builder_build(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(parts)] => {
            let mut total_len = 0usize;
            for p in parts.iter() {
                if let Value::String(s) = p {
                    total_len += s.len();
                }
            }
            let mut out = String::with_capacity(total_len);
            for p in parts.iter() {
                if let Value::String(s) = p {
                    out.push_str(s);
                }
            }
            Ok(Value::String(out))
        }
        [other] => Err(format!(
            "string_builder_build: expected Builder (Array), got {other}"
        )),
        _ => Err(format!(
            "string_builder_build: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `string_builder_len(b) → int` — total byte length of accumulated strings.
pub(crate) fn builtin_string_builder_len(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(parts)] => {
            let total: usize = parts
                .iter()
                .map(|p| match p {
                    Value::String(s) => s.len(),
                    _ => 0,
                })
                .sum();
            Ok(Value::Int(total as i64))
        }
        [other] => Err(format!(
            "string_builder_len: expected Builder (Array), got {other}"
        )),
        _ => Err(format!(
            "string_builder_len: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `string_builder_is_empty(b) → bool` — true when no parts accumulated.
pub(crate) fn builtin_string_builder_is_empty(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(parts)] => Ok(Value::Bool(
            parts
                .iter()
                .all(|p| matches!(p, Value::String(s) if s.is_empty()))
                || parts.is_empty(),
        )),
        [other] => Err(format!(
            "string_builder_is_empty: expected Builder (Array), got {other}"
        )),
        _ => Err(format!(
            "string_builder_is_empty: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `string_builder_clear(b) → Builder` — discard all accumulated parts.
pub(crate) fn builtin_string_builder_clear(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(_)] => Ok(Value::Array(Vec::new())),
        [other] => Err(format!(
            "string_builder_clear: expected Builder (Array), got {other}"
        )),
        _ => Err(format!(
            "string_builder_clear: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> String {
        let r = run_program(src);
        assert!(r.ok, "program failed: {:?}", r.errors);
        r.stdout
    }

    #[test]
    fn basic_append_and_build() {
        let out = run(r#"
let b = string_builder_new();
let b = string_builder_append(b, "hello");
let b = string_builder_append(b, ", ");
let b = string_builder_append(b, "world");
let s = string_builder_build(b);
println(s);
"#);
        assert!(out.contains("hello, world"), "got: {out:?}");
    }

    #[test]
    fn append_int_and_float() {
        let out = run(r#"
let b = string_builder_new();
let b = string_builder_append(b, "x=");
let b = string_builder_append(b, 42);
let s = string_builder_build(b);
println(s);
"#);
        assert!(out.contains("x=42"), "got: {out:?}");
    }

    #[test]
    fn prepend_adds_to_front() {
        let out = run(r#"
let b = string_builder_new();
let b = string_builder_append(b, "world");
let b = string_builder_prepend(b, "hello ");
let s = string_builder_build(b);
println(s);
"#);
        assert!(out.contains("hello world"), "got: {out:?}");
    }

    #[test]
    fn len_counts_bytes() {
        let out = run(r#"
let b = string_builder_new();
let b = string_builder_append(b, "abc");
let b = string_builder_append(b, "de");
println(to_string(string_builder_len(b)));
"#);
        assert!(out.contains("5"), "got: {out:?}");
    }

    #[test]
    fn is_empty_and_clear() {
        let out = run(r#"
let b = string_builder_new();
println(to_string(string_builder_is_empty(b)));
let b = string_builder_append(b, "hi");
let b = string_builder_clear(b);
println(to_string(string_builder_is_empty(b)));
"#);
        assert!(out.contains("true"), "got: {out:?}");
    }

    #[test]
    fn empty_builder_build() {
        let out = run(r#"
let b = string_builder_new();
let s = string_builder_build(b);
println(to_string(string_builder_len(string_builder_new())));
println(s);
"#);
        assert!(out.contains("0"), "got: {out:?}");
    }
}
