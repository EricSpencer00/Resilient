//! RES-2652: Runtime type introspection and collection ergonomics.
//!
//! * `type_of(x) -> string` — returns the runtime type name of `x`.
//! * `result_collect(arr) -> Result` — fold an Array of Results into a
//!   Result<Array>; short-circuits on the first Err.
//! * `array_from_fn(n, fn)` — build an Array of `n` elements by calling
//!   `fn(i)` for `i` in `0..(n-1)`.

use crate::{Interpreter, Value};

type RResult<T> = Result<T, String>;

/// `type_of(x) -> string`
///
/// Returns the runtime type of `x` as a string:
/// `"int"`, `"float"`, `"string"`, `"bool"`, `"array"`, `"map"`, `"set"`,
/// `"void"`, `"function"`, `"bytes"`, `"struct"`, `"tuple"`, `"enum"`,
/// `"result"`, `"option"`, `"actor_pid"`, `"null"`.
///
/// ```text
/// type_of(42)        // == "int"
/// type_of(3.14)      // == "float"
/// type_of([1,2])     // == "array"
/// type_of(Ok(1))     // == "result"
/// type_of(Some(1))   // == "option"
/// ```
pub(crate) fn builtin_type_of(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let name = match v {
                Value::Int(_) => "int",
                Value::Float(_) => "float",
                Value::String(_) => "string",
                Value::Bool(_) => "bool",
                Value::Array(_) => "array",
                Value::Map(_) => "map",
                Value::Set(_) => "set",
                Value::Void => "void",
                Value::Function(_) | Value::Closure { .. } | Value::Builtin { .. } => "function",
                #[cfg(feature = "ffi")]
                Value::Foreign { .. } => "function",
                Value::Bytes(_) => "bytes",
                Value::Struct { .. } => "struct",
                Value::Tuple(_) => "tuple",
                Value::EnumVariant { .. } => "enum",
                Value::Result { .. } => "result",
                Value::Option(_) => "option",
                Value::ActorPid(_) => "actor_pid",
                // Control-flow sentinels — not user-visible.
                Value::Return(_)
                | Value::Break
                | Value::Continue
                | Value::BreakLabel(_)
                | Value::ContinueLabel(_) => "void",
                Value::OpaquePtr(_) | Value::Cell(_) => "opaque",
                // RES-2592: TailCall is an internal control-flow sentinel — not user-visible.
                // RES-2592: internal trampoline sentinel — never user-visible.
                Value::TailCall(_) => "void",
            };
            Ok(Value::String(name.to_string()))
        }
        _ => Err(format!("type_of: expected 1 argument, got {}", args.len())),
    }
}

pub(crate) fn builtin_struct_name(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Struct { name, .. }] => Ok(Value::String(name.clone())),
        [Value::EnumVariant { variant, .. }] => Ok(Value::String(variant.clone())),
        [_] => Err("struct_name: argument is not a struct or enum variant".to_string()),
        _ => Err(format!(
            "struct_name: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `result_collect(arr) -> Result`
///
/// Takes an Array of `Result` values and combines them into a single
/// `Result<Array>`. If all elements are `Ok`, returns `Ok([v1, v2, ...])`.
/// If any element is `Err(e)`, returns that `Err` immediately (short-circuit).
/// Errors if any element is not a `Result`.
///
/// ```text
/// result_collect([Ok(1), Ok(2), Ok(3)])   // == Ok([1, 2, 3])
/// result_collect([Ok(1), Err("bad")])      // == Err("bad")
/// ```
pub(crate) fn builtin_result_collect(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arr)] => {
            let mut out = Vec::with_capacity(arr.len());
            for (i, elem) in arr.iter().enumerate() {
                match elem {
                    Value::Result { ok: true, payload } => {
                        out.push((**payload).clone());
                    }
                    err @ Value::Result { ok: false, .. } => {
                        return Ok(err.clone());
                    }
                    other => {
                        return Err(format!(
                            "result_collect: element at index {i} must be a Result, got {other}"
                        ));
                    }
                }
            }
            Ok(Value::Result {
                ok: true,
                payload: Box::new(Value::Array(out)),
            })
        }
        [other] => Err(format!(
            "result_collect: expected an Array of Results, got {other}"
        )),
        _ => Err(format!(
            "result_collect: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_from_fn(n, fn) -> Array`
///
/// Creates an array of `n` elements by calling `fn(i)` for each index
/// `i` in `0..n`. `n` must be >= 0.
///
/// ```text
/// let squares = array_from_fn(5, fn(int i) -> int { return i * i; });
/// // squares == [0, 1, 4, 9, 16]
/// ```
pub(crate) fn builtin_array_from_fn(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (n, f) = match args {
        [Value::Int(n), f] => (*n, f.clone()),
        [n, _] => {
            return Err(format!(
                "array_from_fn: first argument must be an int, got {n}"
            ));
        }
        _ => {
            return Err(format!(
                "array_from_fn: expected 2 arguments (n, fn), got {}",
                args.len()
            ));
        }
    };

    if n < 0 {
        return Err(format!("array_from_fn: n must be >= 0, got {n}"));
    }

    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        out.push(interp.apply_function(&f, vec![Value::Int(i)])?);
    }
    Ok(Value::Array(out))
}

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── type_of ───────────────────────────────────────────────────────────────

    #[test]
    fn type_of_primitives() {
        let r = run(r#"println(type_of(42));
println(type_of(3.14));
println(type_of("hi"));
println(type_of(true));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "int");
        assert_eq!(lines[1], "float");
        assert_eq!(lines[2], "string");
        assert_eq!(lines[3], "bool");
    }

    #[test]
    fn type_of_collections() {
        let r = run(r#"println(type_of([1,2,3]));
println(type_of({"a" -> 1}));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "array");
        assert_eq!(lines[1], "map");
    }

    #[test]
    fn type_of_result_and_option() {
        let r = run(r#"println(type_of(Ok(1)));
println(type_of(Err("e")));
println(type_of(Some(5)));
println(type_of(None));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "result");
        assert_eq!(lines[1], "result");
        assert_eq!(lines[2], "option");
        assert_eq!(lines[3], "option");
    }

    #[test]
    fn type_of_function() {
        let r = run(r#"let f = fn(int x) -> int { return x; };
println(type_of(f));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("function"), "stdout: {}", r.stdout);
    }

    // ── result_collect ────────────────────────────────────────────────────────

    #[test]
    fn result_collect_all_ok() {
        let r = run(r#"let r = result_collect([Ok(1), Ok(2), Ok(3)]);
println(is_ok(r));
let arr = unwrap(r);
println(arr[0]);
println(arr[2]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "1");
        assert_eq!(lines[2], "3");
    }

    #[test]
    fn result_collect_short_circuits_on_err() {
        let r = run(r#"let r = result_collect([Ok(1), Err("oops"), Ok(3)]);
println(is_err(r));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[test]
    fn result_collect_empty_array() {
        let r = run(r#"let r = result_collect([]);
println(is_ok(r));
let arr = unwrap(r);
println(len(arr));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "0");
    }

    #[test]
    fn result_collect_rejects_non_result_elements() {
        let r = run(r#"let r = result_collect([1, 2, 3]);
println(r);"#);
        assert!(!r.ok, "expected error for non-Result elements");
    }

    // ── array_from_fn ─────────────────────────────────────────────────────────

    #[test]
    fn array_from_fn_squares() {
        let r = run(
            r#"let squares = array_from_fn(5, fn(int i) -> int { return i * i; });
println(squares[0]);
println(squares[1]);
println(squares[4]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "0");
        assert_eq!(lines[1], "1");
        assert_eq!(lines[2], "16");
    }

    #[test]
    fn array_from_fn_zero_length() {
        let r = run(
            r#"let arr = array_from_fn(0, fn(int i) -> int { return i; });
println(len(arr));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'));
    }

    #[test]
    fn array_from_fn_strings() {
        let r = run(
            r#"let arr = array_from_fn(3, fn(int i) -> string { return "item_" + to_string(i); });
println(arr[0]);
println(arr[2]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "item_0");
        assert_eq!(lines[1], "item_2");
    }

    #[test]
    fn array_from_fn_negative_n_errors() {
        let r = run(
            r#"let arr = array_from_fn(-1, fn(int i) -> int { return i; });
println(arr);"#,
        );
        assert!(!r.ok, "expected error for negative n");
    }
}
