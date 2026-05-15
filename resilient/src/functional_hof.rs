//! RES-2656: Functional higher-order function builtins.
//!
//! * `identity(x)` — returns x unchanged.
//! * `const_fn(x)` — returns a closure that always returns x.
//! * `flip(f)` — returns a 2-arg function with arguments swapped.
//! * `array_apply_n(arr, f, n)` — apply f to each element n times (uses existing array_iterate).
//! * `array_apply_n(arr, f, n)` — apply f to each element n times.
//! * `array_zip_with_fn(a, b, f)` — zip two arrays and apply f(a_i, b_i).
//! * `array_scan_fn(arr, init, f)` — running fold returning all intermediate results.
//! * `array_flat_map_fn(arr, f)` — map then flatten (standalone, any callback).

use crate::{Interpreter, Value};

type RResult<T> = Result<T, String>;

/// `identity(x) -> x`
///
/// Returns `x` unchanged. Useful as a no-op placeholder in HOF pipelines.
///
/// ```text
/// identity(42)       // == 42
/// identity("hello")  // == "hello"
/// ```
pub(crate) fn builtin_identity(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(v.clone()),
        _ => Err(format!("identity: expected 1 argument, got {}", args.len())),
    }
}

/// `array_zip_with_fn(a, b, f) -> Array`
///
/// Zips two arrays and applies `f(a_i, b_i)` for each pair of elements.
/// The arrays must have the same length.
///
/// ```text
/// array_zip_with_fn([1,2,3], [4,5,6], fn(int x, int y) -> int { return x + y; })
/// // == [5, 7, 9]
/// ```
pub(crate) fn builtin_array_zip_with_fn(
    interp: &mut Interpreter,
    args: &[Value],
) -> RResult<Value> {
    match args {
        [Value::Array(a), Value::Array(b), f] => {
            if a.len() != b.len() {
                return Err(format!(
                    "array_zip_with_fn: arrays have different lengths ({} vs {})",
                    a.len(),
                    b.len()
                ));
            }
            let f = f.clone();
            let pairs: Vec<(Value, Value)> = a
                .iter()
                .zip(b.iter())
                .map(|(x, y)| (x.clone(), y.clone()))
                .collect();
            let mut out = Vec::with_capacity(pairs.len());
            for (x, y) in pairs {
                out.push(interp.apply_function(f.clone(), vec![x, y])?);
            }
            Ok(Value::Array(out))
        }
        [a, _, _] if !matches!(a, Value::Array(_)) => Err(format!(
            "array_zip_with_fn: first argument must be Array, got {a}"
        )),
        [_, b, _] if !matches!(b, Value::Array(_)) => Err(format!(
            "array_zip_with_fn: second argument must be Array, got {b}"
        )),
        _ => Err(format!(
            "array_zip_with_fn: expected 3 arguments (a, b, f), got {}",
            args.len()
        )),
    }
}

/// `array_scan_fn(arr, init, f) -> Array`
///
/// Performs a running fold using `f(acc, elem)` and returns an array of
/// all intermediate accumulator values (including `init`). Length is
/// `len(arr) + 1`.
///
/// ```text
/// array_scan_fn([1,2,3,4], 0, fn(int acc, int x) -> int { return acc + x; })
/// // == [0, 1, 3, 6, 10]
/// ```
pub(crate) fn builtin_array_scan_fn(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arr), init, f] => {
            let f = f.clone();
            let elems: Vec<Value> = arr.clone();
            let mut out = Vec::with_capacity(elems.len() + 1);
            let mut acc = init.clone();
            out.push(acc.clone());
            for elem in elems {
                acc = interp.apply_function(f.clone(), vec![acc, elem])?;
                out.push(acc.clone());
            }
            Ok(Value::Array(out))
        }
        [other, _, _] if !matches!(other, Value::Array(_)) => Err(format!(
            "array_scan_fn: first argument must be Array, got {other}"
        )),
        _ => Err(format!(
            "array_scan_fn: expected 3 arguments (arr, init, f), got {}",
            args.len()
        )),
    }
}

/// `array_flat_map_fn(arr, f) -> Array`
///
/// Maps `f` over each element of `arr`, expecting `f` to return an Array,
/// and concatenates all resulting arrays into a single flat Array.
///
/// ```text
/// array_flat_map_fn([1,2,3], fn(int x) -> IntArr { return [x, x*x]; })
/// // == [1, 1, 2, 4, 3, 9]
/// ```
pub(crate) fn builtin_array_flat_map_fn(
    interp: &mut Interpreter,
    args: &[Value],
) -> RResult<Value> {
    match args {
        [Value::Array(arr), f] => {
            let f = f.clone();
            let elems: Vec<Value> = arr.clone();
            let mut out = Vec::new();
            for (i, elem) in elems.into_iter().enumerate() {
                match interp.apply_function(f.clone(), vec![elem])? {
                    Value::Array(sub) => out.extend(sub),
                    other => {
                        return Err(format!(
                            "array_flat_map_fn: f must return Array; got {other} at index {i}"
                        ));
                    }
                }
            }
            Ok(Value::Array(out))
        }
        [other, _] if !matches!(other, Value::Array(_)) => Err(format!(
            "array_flat_map_fn: first argument must be Array, got {other}"
        )),
        _ => Err(format!(
            "array_flat_map_fn: expected 2 arguments (arr, f), got {}",
            args.len()
        )),
    }
}

/// `array_apply_n(arr, f, n) -> Array`
///
/// Applies `f` to each element of `arr` exactly `n` times (chained).
/// Equivalent to `array_map(arr, fn(x) { array_iterate(x, f, n) })`.
///
/// ```text
/// array_apply_n([1,2,3], fn(int x) -> int { return x * 2; }, 3)
/// // == [8, 16, 24]   (each doubled 3 times)
/// ```
pub(crate) fn builtin_array_apply_n(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arr), f, Value::Int(n)] => {
            if *n < 0 {
                return Err(format!("array_apply_n: n must be >= 0, got {n}"));
            }
            let n = *n;
            let f = f.clone();
            let elems: Vec<Value> = arr.clone();
            let mut out = Vec::with_capacity(elems.len());
            for elem in elems {
                let mut v = elem;
                for _ in 0..n {
                    v = interp.apply_function(f.clone(), vec![v])?;
                }
                out.push(v);
            }
            Ok(Value::Array(out))
        }
        [Value::Array(_), _, n] => Err(format!(
            "array_apply_n: third argument must be int, got {n}"
        )),
        [other, _, _] => Err(format!(
            "array_apply_n: first argument must be Array, got {other}"
        )),
        _ => Err(format!(
            "array_apply_n: expected 3 arguments (arr, f, n), got {}",
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

    // ── identity ──────────────────────────────────────────────────────────────

    #[test]
    fn identity_int() {
        let r = run("println(identity(42));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("42"), "stdout: {}", r.stdout);
    }

    #[test]
    fn identity_string() {
        let r = run(r#"println(identity("hello"));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("hello"), "stdout: {}", r.stdout);
    }

    #[test]
    fn identity_array() {
        let r = run("println(identity([1,2,3]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[1, 2, 3]"), "stdout: {}", r.stdout);
    }

    // ── array_iterate (uses existing collection_extras builtin: init, n, fn) ──

    #[test]
    fn array_iterate_double_5_times() {
        let r = run(r#"let f = fn(int x) -> int { return x * 2; };
println(array_iterate(1, 5, f));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("32"), "stdout: {}", r.stdout);
    }

    #[test]
    fn array_iterate_zero_times() {
        let r = run(r#"let f = fn(int x) -> int { return x + 1; };
println(array_iterate(10, 0, f));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("10"), "stdout: {}", r.stdout);
    }

    #[test]
    fn array_iterate_string_append() {
        let r = run(r#"let f = fn(string s) -> string { return s + "!"; };
println(array_iterate("hi", 3, f));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("hi!!!"), "stdout: {}", r.stdout);
    }

    // ── array_zip_with_fn ─────────────────────────────────────────────────────

    #[test]
    fn array_zip_with_fn_add() {
        let r = run(r#"let f = fn(int x, int y) -> int { return x + y; };
println(array_zip_with_fn([1,2,3], [4,5,6], f));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[5, 7, 9]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn array_zip_with_fn_pair_as_array() {
        let r = run(r#"let f = fn(int x, int y) -> IntArr { return [x, y]; };
let result = array_zip_with_fn([1,2], [3,4], f);
println(len(result));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('2'), "stdout: {}", r.stdout);
    }

    #[test]
    fn array_zip_with_fn_length_mismatch_errors() {
        let r = run(r#"let f = fn(int x, int y) -> int { return x + y; };
array_zip_with_fn([1,2], [3,4,5], f);"#);
        assert!(!r.ok, "expected error for length mismatch");
    }

    // ── array_scan_fn ─────────────────────────────────────────────────────────

    #[test]
    fn array_scan_fn_running_sum() {
        let r = run(r#"let f = fn(int acc, int x) -> int { return acc + x; };
println(array_scan_fn([1,2,3,4], 0, f));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(
            r.stdout.contains("[0, 1, 3, 6, 10]"),
            "stdout: {}",
            r.stdout
        );
    }

    #[test]
    fn array_scan_fn_empty_array() {
        let r = run(r#"let f = fn(int acc, int x) -> int { return acc + x; };
let result = array_scan_fn([], 99, f);
println(len(result));
println(result[0]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "1");
        assert_eq!(lines[1], "99");
    }

    // ── array_flat_map_fn ─────────────────────────────────────────────────────

    #[test]
    fn array_flat_map_fn_duplicate() {
        let r = run(r#"let f = fn(int x) -> IntArr { return [x, x]; };
println(array_flat_map_fn([1,2,3], f));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(
            r.stdout.contains("[1, 1, 2, 2, 3, 3]"),
            "stdout: {}",
            r.stdout
        );
    }

    #[test]
    fn array_flat_map_fn_expand() {
        let r = run(r#"let f = fn(int x) -> IntArr { return [x, x * x]; };
println(array_flat_map_fn([1,2,3], f));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(
            r.stdout.contains("[1, 1, 2, 4, 3, 9]"),
            "stdout: {}",
            r.stdout
        );
    }

    #[test]
    fn array_flat_map_fn_empty() {
        let r = run(r#"let f = fn(int x) -> IntArr { return []; };
println(len(array_flat_map_fn([1,2,3], f)));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    // ── array_apply_n ─────────────────────────────────────────────────────────

    #[test]
    fn array_apply_n_triple() {
        let r = run(r#"let double = fn(int x) -> int { return x * 2; };
println(array_apply_n([1,2,3], double, 3));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        // 1*8=8, 2*8=16, 3*8=24
        assert!(r.stdout.contains("[8, 16, 24]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn array_apply_n_zero() {
        let r = run(r#"let double = fn(int x) -> int { return x * 2; };
println(array_apply_n([5, 10], double, 0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[5, 10]"), "stdout: {}", r.stdout);
    }

    // ── integration: using HOFs together ──────────────────────────────────────

    #[test]
    fn scan_then_last_element() {
        // Running sum of [1..5], last element should be 15
        let r = run(r#"let add = fn(int a, int b) -> int { return a + b; };
let sums = array_scan_fn([1,2,3,4,5], 0, add);
println(sums[5]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("15"), "stdout: {}", r.stdout);
    }

    #[test]
    fn flat_map_then_zip() {
        // flat_map([1,2], x->[x,-x]) => [1,-1,2,-2]
        // then zip_with sum => but we just check length here
        let r = run(r#"let expand = fn(int x) -> IntArr { return [x, 0 - x]; };
let result = array_flat_map_fn([1, 2], expand);
println(len(result));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('4'), "stdout: {}", r.stdout);
    }
}
