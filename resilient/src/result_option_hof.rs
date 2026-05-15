//! RES-2651: Higher-order operations on Result and Option values.
//!
//! Result operations:
//! * `result_map(r, fn)`        — if Ok(v) → Ok(fn(v)); Err passthrough.
//! * `result_and_then(r, fn)`   — if Ok(v) → fn(v) (must return Result); Err passthrough.
//! * `result_map_err(r, fn)`    — if Err(e) → Err(fn(e)); Ok passthrough.
//! * `result_or_else(r, fn)`    — if Err(e) → fn(e); Ok passthrough.
//!
//! Option operations:
//! * `option_map(o, fn)`        — if Some(v) → Some(fn(v)); None passthrough.
//! * `option_and_then(o, fn)`   — if Some(v) → fn(v) (must return Option); None passthrough.
//! * `option_filter(o, fn)`     — if Some(v) and fn(v) → keep; else None.
//! * `option_or_else(o, fn)`    — if None → fn() (must return Option); Some passthrough.
//! * `option_ok_or(o, err_val)` — convert Option to Result: Some(v) → Ok(v); None → Err(err_val).

use crate::{Interpreter, Value};

type RResult<T> = Result<T, String>;

// ─────────────────────────────────── Result ───────────────────────────────────

/// `result_map(r, fn) -> Result`
///
/// If `r` is `Ok(v)`, returns `Ok(fn(v))`. If `r` is `Err(e)`, returns it
/// unchanged. Analogous to `Result::map` in Rust.
pub(crate) fn builtin_result_map(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (r, f) = match args {
        [Value::Result { .. }, f] => (args[0].clone(), f.clone()),
        [a, _] => {
            return Err(format!(
                "result_map: first argument must be a Result, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "result_map: expected 2 arguments (result, fn), got {}",
                args.len()
            ));
        }
    };
    match r {
        Value::Result { ok: true, payload } => {
            let mapped = interp.apply_function(f, vec![*payload])?;
            Ok(Value::Result {
                ok: true,
                payload: Box::new(mapped),
            })
        }
        err @ Value::Result { ok: false, .. } => Ok(err),
        _ => unreachable!(),
    }
}

/// `result_and_then(r, fn) -> Result`
///
/// If `r` is `Ok(v)`, calls `fn(v)` and returns its result (which must be a
/// `Result`). If `r` is `Err(e)`, returns it unchanged. This is the monadic
/// bind / flatMap for `Result`.
pub(crate) fn builtin_result_and_then(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (r, f) = match args {
        [Value::Result { .. }, f] => (args[0].clone(), f.clone()),
        [a, _] => {
            return Err(format!(
                "result_and_then: first argument must be a Result, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "result_and_then: expected 2 arguments (result, fn), got {}",
                args.len()
            ));
        }
    };
    match r {
        Value::Result { ok: true, payload } => {
            let out = interp.apply_function(f, vec![*payload])?;
            match out {
                r @ Value::Result { .. } => Ok(r),
                other => Err(format!(
                    "result_and_then: callback must return a Result, got {other}"
                )),
            }
        }
        err @ Value::Result { ok: false, .. } => Ok(err),
        _ => unreachable!(),
    }
}

/// `result_map_err(r, fn) -> Result`
///
/// If `r` is `Err(e)`, returns `Err(fn(e))`. If `r` is `Ok(v)`, returns it
/// unchanged.
pub(crate) fn builtin_result_map_err(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (r, f) = match args {
        [Value::Result { .. }, f] => (args[0].clone(), f.clone()),
        [a, _] => {
            return Err(format!(
                "result_map_err: first argument must be a Result, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "result_map_err: expected 2 arguments (result, fn), got {}",
                args.len()
            ));
        }
    };
    match r {
        ok @ Value::Result { ok: true, .. } => Ok(ok),
        Value::Result { ok: false, payload } => {
            let mapped = interp.apply_function(f, vec![*payload])?;
            Ok(Value::Result {
                ok: false,
                payload: Box::new(mapped),
            })
        }
        _ => unreachable!(),
    }
}

/// `result_or_else(r, fn) -> Result`
///
/// If `r` is `Err(e)`, calls `fn(e)` and returns its result (which must be a
/// `Result`). If `r` is `Ok(v)`, returns it unchanged.
pub(crate) fn builtin_result_or_else(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (r, f) = match args {
        [Value::Result { .. }, f] => (args[0].clone(), f.clone()),
        [a, _] => {
            return Err(format!(
                "result_or_else: first argument must be a Result, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "result_or_else: expected 2 arguments (result, fn), got {}",
                args.len()
            ));
        }
    };
    match r {
        ok @ Value::Result { ok: true, .. } => Ok(ok),
        Value::Result { ok: false, payload } => {
            let out = interp.apply_function(f, vec![*payload])?;
            match out {
                r @ Value::Result { .. } => Ok(r),
                other => Err(format!(
                    "result_or_else: callback must return a Result, got {other}"
                )),
            }
        }
        _ => unreachable!(),
    }
}

// ─────────────────────────────────── Option ───────────────────────────────────

/// `option_map(o, fn) -> Option`
///
/// If `o` is `Some(v)`, returns `Some(fn(v))`. If `o` is `None`, returns
/// `None`. Analogous to `Option::map` in Rust.
pub(crate) fn builtin_option_map(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (o, f) = match args {
        [Value::Option(_), f] => (args[0].clone(), f.clone()),
        [a, _] => {
            return Err(format!(
                "option_map: first argument must be an Option, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "option_map: expected 2 arguments (option, fn), got {}",
                args.len()
            ));
        }
    };
    match o {
        Value::Option(Some(inner)) => {
            let mapped = interp.apply_function(f, vec![*inner])?;
            Ok(Value::Option(Some(Box::new(mapped))))
        }
        none @ Value::Option(None) => Ok(none),
        _ => unreachable!(),
    }
}

/// `option_and_then(o, fn) -> Option`
///
/// If `o` is `Some(v)`, calls `fn(v)` and returns its result (which must be
/// an `Option`). If `o` is `None`, returns `None`. Monadic bind for `Option`.
pub(crate) fn builtin_option_and_then(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (o, f) = match args {
        [Value::Option(_), f] => (args[0].clone(), f.clone()),
        [a, _] => {
            return Err(format!(
                "option_and_then: first argument must be an Option, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "option_and_then: expected 2 arguments (option, fn), got {}",
                args.len()
            ));
        }
    };
    match o {
        Value::Option(Some(inner)) => {
            let out = interp.apply_function(f, vec![*inner])?;
            match out {
                o @ Value::Option(_) => Ok(o),
                other => Err(format!(
                    "option_and_then: callback must return an Option, got {other}"
                )),
            }
        }
        none @ Value::Option(None) => Ok(none),
        _ => unreachable!(),
    }
}

/// `option_filter(o, fn) -> Option`
///
/// If `o` is `Some(v)` and `fn(v)` returns `true`, returns `Some(v)`. In all
/// other cases (None, or predicate returned false) returns `None`.
pub(crate) fn builtin_option_filter(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (o, f) = match args {
        [Value::Option(_), f] => (args[0].clone(), f.clone()),
        [a, _] => {
            return Err(format!(
                "option_filter: first argument must be an Option, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "option_filter: expected 2 arguments (option, fn), got {}",
                args.len()
            ));
        }
    };
    match o {
        Value::Option(Some(inner)) => match interp.apply_function(f, vec![*inner.clone()])? {
            Value::Bool(true) => Ok(Value::Option(Some(inner))),
            Value::Bool(false) => Ok(Value::Option(None)),
            other => Err(format!(
                "option_filter: predicate must return bool, got {other}"
            )),
        },
        none @ Value::Option(None) => Ok(none),
        _ => unreachable!(),
    }
}

/// `option_or_else(o, fn) -> Option`
///
/// If `o` is `None`, calls `fn()` (no arguments) and returns the result (which
/// must be an `Option`). If `o` is `Some(v)`, returns it unchanged.
pub(crate) fn builtin_option_or_else(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (o, f) = match args {
        [Value::Option(_), f] => (args[0].clone(), f.clone()),
        [a, _] => {
            return Err(format!(
                "option_or_else: first argument must be an Option, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "option_or_else: expected 2 arguments (option, fn), got {}",
                args.len()
            ));
        }
    };
    match o {
        some @ Value::Option(Some(_)) => Ok(some),
        Value::Option(None) => {
            let out = interp.apply_function(f, vec![])?;
            match out {
                o @ Value::Option(_) => Ok(o),
                other => Err(format!(
                    "option_or_else: callback must return an Option, got {other}"
                )),
            }
        }
        _ => unreachable!(),
    }
}

/// `option_ok_or(o, err_val) -> Result`
///
/// Converts an `Option` to a `Result`: `Some(v)` becomes `Ok(v)` and `None`
/// becomes `Err(err_val)`.
pub(crate) fn builtin_option_ok_or(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Option(Some(inner)), _] => Ok(Value::Result {
            ok: true,
            payload: inner.clone(),
        }),
        [Value::Option(None), err_val] => Ok(Value::Result {
            ok: false,
            payload: Box::new(err_val.clone()),
        }),
        [a, _] => Err(format!(
            "option_ok_or: first argument must be an Option, got {a}"
        )),
        _ => Err(format!(
            "option_ok_or: expected 2 arguments (option, err_val), got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── result_map ────────────────────────────────────────────────────────────

    #[test]
    fn result_map_ok_transforms_value() {
        let r = run(r#"let r = Ok(5);
let r2 = result_map(r, fn(int x) -> int { return x * 2; });
println(is_ok(r2));
println(unwrap(r2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "10");
    }

    #[test]
    fn result_map_err_passes_through() {
        let r = run(r#"let r = Err("oops");
let r2 = result_map(r, fn(int x) -> int { return x * 2; });
println(is_err(r2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    // ── result_and_then ───────────────────────────────────────────────────────

    #[test]
    fn result_and_then_chains_ok() {
        let r = run(r#"let r = Ok(10);
let r2 = result_and_then(r, fn(int x) -> Result {
    if x > 5 { return Ok(x + 100); }
    return Err("too small");
});
println(unwrap(r2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("110"), "stdout: {}", r.stdout);
    }

    #[test]
    fn result_and_then_short_circuits_on_err() {
        let r = run(r#"let r = Err("initial");
let r2 = result_and_then(r, fn(int x) -> Result { return Ok(x); });
println(is_err(r2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    // ── result_map_err ────────────────────────────────────────────────────────

    #[test]
    fn result_map_err_transforms_error() {
        let r = run(r#"let r = Err(404);
let r2 = result_map_err(r, fn(int code) -> string { return "HTTP " + to_string(code); });
println(is_err(r2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[test]
    fn result_map_err_ok_passes_through() {
        let r = run(r#"let r = Ok(42);
let r2 = result_map_err(r, fn(string e) -> string { return "wrapped: " + e; });
println(unwrap(r2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("42"), "stdout: {}", r.stdout);
    }

    // ── result_or_else ────────────────────────────────────────────────────────

    #[test]
    fn result_or_else_recovers_from_err() {
        let r = run(r#"let r = Err(-1);
let r2 = result_or_else(r, fn(int code) -> Result {
    if code < 0 { return Ok(0); }
    return Err(code);
});
println(unwrap(r2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    // ── option_map ────────────────────────────────────────────────────────────

    #[test]
    fn option_map_some_transforms() {
        let r = run(r#"let o = Some(7);
let o2 = option_map(o, fn(int x) -> int { return x * 3; });
println(is_some(o2));
println(option_unwrap(o2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "21");
    }

    #[test]
    fn option_map_none_stays_none() {
        let r = run(r#"let o = None;
let o2 = option_map(o, fn(int x) -> int { return x + 1; });
println(is_none(o2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    // ── option_and_then ───────────────────────────────────────────────────────

    #[test]
    fn option_and_then_chains_some() {
        let r = run(r#"let o = Some(5);
let o2 = option_and_then(o, fn(int x) -> Option {
    if x > 3 { return Some(x * 10); }
    return None;
});
println(option_unwrap(o2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("50"), "stdout: {}", r.stdout);
    }

    #[test]
    fn option_and_then_none_short_circuits() {
        let r = run(r#"let o = None;
let o2 = option_and_then(o, fn(int x) -> Option { return Some(x); });
println(is_none(o2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    // ── option_filter ─────────────────────────────────────────────────────────

    #[test]
    fn option_filter_keeps_matching() {
        let r = run(r#"let o = Some(10);
let o2 = option_filter(o, fn(int x) -> bool { return x > 5; });
println(is_some(o2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[test]
    fn option_filter_removes_non_matching() {
        let r = run(r#"let o = Some(3);
let o2 = option_filter(o, fn(int x) -> bool { return x > 5; });
println(is_none(o2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    // ── option_or_else ────────────────────────────────────────────────────────

    #[test]
    fn option_or_else_provides_fallback() {
        let r = run(r#"let o = None;
let o2 = option_or_else(o, fn() -> Option { return Some(99); });
println(option_unwrap(o2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("99"), "stdout: {}", r.stdout);
    }

    #[test]
    fn option_or_else_some_unchanged() {
        let r = run(r#"let o = Some(42);
let o2 = option_or_else(o, fn() -> Option { return Some(99); });
println(option_unwrap(o2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("42"), "stdout: {}", r.stdout);
    }

    // ── option_ok_or ──────────────────────────────────────────────────────────

    #[test]
    fn option_ok_or_some_gives_ok() {
        let r = run(r#"let o = Some(7);
let result = option_ok_or(o, "missing");
println(is_ok(result));
println(unwrap(result));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "7");
    }

    #[test]
    fn option_ok_or_none_gives_err() {
        let r = run(r#"let o = None;
let result = option_ok_or(o, "not found");
println(is_err(result));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }
}
