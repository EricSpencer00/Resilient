//! RES-2647: Higher-order functional map operations.
//!
//! * `map_filter(m, fn)` — keep only entries where `fn(key, val)` returns true.
//! * `map_map_values(m, fn)` — transform each value via `fn(key, val) -> new_val`.
//! * `map_for_each(m, fn)` — iterate entries (side effects only), returns Void.
//! * `map_to_pairs(m)` — convert Map to Array of 2-element [key, val] Arrays
//!   (inverse of `map_from_pairs`).
//! * `map_invert(m)` — swap keys and values; values must be hashable.

use crate::{Interpreter, MapKey, Value};

type RResult<T> = Result<T, String>;

/// `map_filter(m, fn) -> Map`
///
/// Keeps only the entries of `m` for which `fn(key, value)` returns true.
/// The key is passed as the first argument (as its runtime value — Int/String/Bool)
/// and the value as the second argument.
///
/// ```text
/// let m2 = map_filter(m, fn(string k, int v) -> bool { return v > 0; });
/// ```
pub(crate) fn builtin_map_filter(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (m, f) = match args {
        [Value::Map(m), f] => (m.clone(), f.clone()),
        [a, _] => return Err(format!("map_filter: first argument must be a Map, got {a}")),
        _ => {
            return Err(format!(
                "map_filter: expected 2 arguments (map, fn), got {}",
                args.len()
            ))
        }
    };

    let mut out = std::collections::HashMap::with_capacity(m.len());
    for (k, v) in &m {
        let k_val = map_key_to_value(k);
        let keep = interp.apply_function(f.clone(), vec![k_val, v.clone()])?;
        match keep {
            Value::Bool(true) => {
                out.insert(k.clone(), v.clone());
            }
            Value::Bool(false) => {}
            other => {
                return Err(format!(
                    "map_filter: predicate must return bool, got {other}"
                ))
            }
        }
    }
    Ok(Value::Map(out))
}

/// `map_map_values(m, fn) -> Map`
///
/// Transforms each value in `m` by calling `fn(key, value) -> new_value`.
/// Keys are unchanged; the result map has the same key set.
///
/// ```text
/// let doubled = map_map_values(m, fn(string k, int v) -> int { return v * 2; });
/// ```
pub(crate) fn builtin_map_map_values(
    interp: &mut Interpreter,
    args: &[Value],
) -> RResult<Value> {
    let (m, f) = match args {
        [Value::Map(m), f] => (m.clone(), f.clone()),
        [a, _] => return Err(format!("map_map_values: first argument must be a Map, got {a}")),
        _ => {
            return Err(format!(
                "map_map_values: expected 2 arguments (map, fn), got {}",
                args.len()
            ))
        }
    };

    let mut out = std::collections::HashMap::with_capacity(m.len());
    for (k, v) in &m {
        let k_val = map_key_to_value(k);
        let new_val = interp.apply_function(f.clone(), vec![k_val, v.clone()])?;
        out.insert(k.clone(), new_val);
    }
    Ok(Value::Map(out))
}

/// `map_for_each(m, fn) -> Void`
///
/// Calls `fn(key, value)` for each entry in `m` for side effects. The return
/// value of `fn` is discarded. Useful for logging or accumulating into external
/// mutable state via closures.
///
/// ```text
/// map_for_each(m, fn(string k, int v) -> Void { println(k); });
/// ```
pub(crate) fn builtin_map_for_each(
    interp: &mut Interpreter,
    args: &[Value],
) -> RResult<Value> {
    let (m, f) = match args {
        [Value::Map(m), f] => (m.clone(), f.clone()),
        [a, _] => return Err(format!("map_for_each: first argument must be a Map, got {a}")),
        _ => {
            return Err(format!(
                "map_for_each: expected 2 arguments (map, fn), got {}",
                args.len()
            ))
        }
    };

    for (k, v) in &m {
        let k_val = map_key_to_value(k);
        interp.apply_function(f.clone(), vec![k_val, v.clone()])?;
    }
    Ok(Value::Void)
}

/// `map_to_pairs(m) -> Array`
///
/// Converts `m` to an Array of 2-element Arrays `[[key, val], ...]`. The
/// inverse of `map_from_pairs`. Iteration order is unspecified (HashMap).
///
/// ```text
/// let pairs = map_to_pairs({"a" -> 1, "b" -> 2});
/// // pairs contains ["a", 1] and ["b", 2] in some order
/// ```
pub(crate) fn builtin_map_to_pairs(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Map(m)] => {
            let pairs: Vec<Value> = m
                .iter()
                .map(|(k, v)| Value::Array(vec![map_key_to_value(k), v.clone()]))
                .collect();
            Ok(Value::Array(pairs))
        }
        [other] => Err(format!(
            "map_to_pairs: expected a Map, got {other}"
        )),
        _ => Err(format!(
            "map_to_pairs: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `map_invert(m) -> Map`
///
/// Returns a new Map with keys and values swapped. The values of `m` must be
/// hashable (int, string, bool). If the original map has duplicate values,
/// the result is implementation-defined (last key for each value wins in
/// HashMap iteration order).
///
/// ```text
/// let inv = map_invert({"a" -> 1, "b" -> 2});
/// // inv == {1 -> "a", 2 -> "b"}
/// ```
pub(crate) fn builtin_map_invert(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Map(m)] => {
            let mut out = std::collections::HashMap::with_capacity(m.len());
            for (k, v) in m {
                let new_key = MapKey::from_value(v).map_err(|e| {
                    format!("map_invert: value {v} cannot become a key: {e}")
                })?;
                let new_val = map_key_to_value(k);
                out.insert(new_key, new_val);
            }
            Ok(Value::Map(out))
        }
        [other] => Err(format!("map_invert: expected a Map, got {other}")),
        _ => Err(format!(
            "map_invert: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// Convert a `MapKey` back to a `Value` for passing to callbacks.
fn map_key_to_value(k: &MapKey) -> Value {
    match k {
        MapKey::Int(n) => Value::Int(*n),
        MapKey::Str(s) => Value::String(s.clone()),
        MapKey::Bool(b) => Value::Bool(*b),
    }
}

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── map_filter ────────────────────────────────────────────────────────────

    #[test]
    fn map_filter_keeps_passing_entries() {
        let r = run(
            r#"let m = {"a" -> 10, "b" -> -1, "c" -> 5};
let pos = map_filter(m, fn(string k, int v) -> bool { return v > 0; });
println(map_len(pos));
println(pos["a"]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "2", "expected 2 positive entries: {}", r.stdout);
        assert_eq!(lines[1], "10", "a=10: {}", r.stdout);
    }

    #[test]
    fn map_filter_empty_result() {
        let r = run(
            r#"let m = {"x" -> 1};
let none = map_filter(m, fn(string k, int v) -> bool { return false; });
println(map_len(none));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "empty: {}", r.stdout);
    }

    #[test]
    fn map_filter_rejects_non_bool_predicate() {
        let r = run(
            r#"let m = {"a" -> 1};
let bad = map_filter(m, fn(string k, int v) -> int { return v; });
println(bad);"#,
        );
        assert!(!r.ok, "expected error for non-bool predicate");
    }

    // ── map_map_values ────────────────────────────────────────────────────────

    #[test]
    fn map_map_values_doubles_values() {
        let r = run(
            r#"let m = {"a" -> 3, "b" -> 7};
let doubled = map_map_values(m, fn(string k, int v) -> int { return v * 2; });
println(doubled["a"]);
println(doubled["b"]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('6'), "a*2=6: {}", r.stdout);
        assert!(r.stdout.contains("14"), "b*2=14: {}", r.stdout);
    }

    #[test]
    fn map_map_values_preserves_key_count() {
        let r = run(
            r#"let m = {"x" -> 1, "y" -> 2, "z" -> 3};
let t = map_map_values(m, fn(string k, int v) -> string { return k; });
println(map_len(t));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "3 entries: {}", r.stdout);
    }

    // ── map_for_each ──────────────────────────────────────────────────────────

    #[test]
    fn map_for_each_visits_all_entries() {
        let r = run(
            r#"let m = {"p" -> 10, "q" -> 20};
let total = [0];
map_for_each(m, fn(string k, int v) -> Void { total[0] = total[0] + v; });
println(total[0]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("30"), "sum=30: {}", r.stdout);
    }

    // ── map_to_pairs ──────────────────────────────────────────────────────────

    #[test]
    fn map_to_pairs_length_matches() {
        let r = run(
            r#"let m = {"a" -> 1, "b" -> 2, "c" -> 3};
let pairs = map_to_pairs(m);
println(len(pairs));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "3 pairs: {}", r.stdout);
    }

    #[test]
    fn map_to_pairs_roundtrips_via_from_pairs() {
        let r = run(
            r#"let m = {"hello" -> 42};
let pairs = map_to_pairs(m);
let m2 = map_from_pairs(pairs);
println(m2["hello"]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("42"), "roundtrip: {}", r.stdout);
    }

    // ── map_invert ────────────────────────────────────────────────────────────

    #[test]
    fn map_invert_swaps_keys_and_values() {
        let r = run(
            r#"let m = {"a" -> 1, "b" -> 2};
let inv = map_invert(m);
println(inv[1]);
println(inv[2]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('a'), "1->a: {}", r.stdout);
        assert!(r.stdout.contains('b'), "2->b: {}", r.stdout);
    }

    #[test]
    fn map_invert_rejects_non_hashable_value() {
        let r = run(
            r#"let m = {"x" -> 1.5};
let inv = map_invert(m);
println(inv);"#,
        );
        assert!(!r.ok, "expected error for float value as key");
    }
}
