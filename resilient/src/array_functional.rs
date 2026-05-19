//! RES-2646: Higher-order functional array operations.
//!
//! * `array_flat_map(arr, fn)` — apply `fn` to each element (must return an
//!   Array), then flatten one level. Equivalent to `flatMap` / `concatMap`.
//! * `array_group_by(arr, fn)` — partition `arr` into a Map<key → Array>
//!   where `fn(elem)` provides the key (must be int/string/bool hashable).
//! * `array_partition(arr, fn)` — split into `[[passing], [failing]]` using
//!   a boolean predicate `fn(elem) -> bool`.
//! * `map_from_pairs(pairs)` — build a Map from an Array of 2-element
//!   Arrays `[[key, val], ...]`.
//! * `array_scan(arr, init, fn)` — like `array_reduce` but returns all
//!   intermediate accumulator values (including the initial value).

use crate::{Interpreter, MapKey, Value};

type RResult<T> = Result<T, String>;

/// `array_flat_map(arr, fn) -> Array`
///
/// Applies `fn` to each element of `arr`; every call must return an Array.
/// The results are concatenated into a single flat array (one level of
/// flattening only). Equivalent to `array_flatten(array_map(arr, fn))` but
/// done in a single pass.
///
/// ```text
/// let result = array_flat_map([1, 2, 3], fn(int x) -> Array { [x, x * 10] });
/// // result == [1, 10, 2, 20, 3, 30]
/// ```
pub(crate) fn builtin_array_flat_map(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (arr, f) = match args {
        [Value::Array(a), f] => (a.clone(), f.clone()),
        [a, _] => {
            return Err(format!(
                "array_flat_map: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_flat_map: expected 2 arguments (array, fn), got {}",
                args.len()
            ));
        }
    };

    let mut out: Vec<Value> = Vec::with_capacity(arr.len() * 2);
    for elem in arr {
        let result = interp.apply_function(&f, vec![elem])?;
        match result {
            Value::Array(inner) => out.extend(inner),
            other => {
                return Err(format!(
                    "array_flat_map: callback must return an Array, got {other}"
                ));
            }
        }
    }
    Ok(Value::Array(out))
}

/// `array_group_by(arr, fn) -> Map`
///
/// Groups elements of `arr` by the key returned by `fn(elem)`. The key must
/// be int, string, or bool (hashable). Returns a Map whose values are Arrays
/// of elements that share the same key; order within each group is preserved.
///
/// ```text
/// let m = array_group_by([1,2,3,4], fn(int x) -> string {
///     if x % 2 == 0 { "even" } else { "odd" }
/// });
/// // m == {"even" -> [2, 4], "odd" -> [1, 3]}
/// ```
pub(crate) fn builtin_array_group_by(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (arr, f) = match args {
        [Value::Array(a), f] => (a.clone(), f.clone()),
        [a, _] => {
            return Err(format!(
                "array_group_by: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_group_by: expected 2 arguments (array, fn), got {}",
                args.len()
            ));
        }
    };

    // Use an IndexMap-style approach: insertion-ordered map for deterministic output.
    let mut order: Vec<MapKey> = Vec::new();
    let mut groups: std::collections::HashMap<MapKey, Vec<Value>> =
        std::collections::HashMap::new();

    for elem in arr {
        let key_val = interp.apply_function(&f, vec![elem.clone()])?;
        let mk = MapKey::from_value(&key_val).map_err(|e| {
            format!("array_group_by: key function returned non-hashable value: {e}")
        })?;
        if !groups.contains_key(&mk) {
            order.push(mk.clone());
            groups.insert(mk.clone(), Vec::new());
        }
        groups.get_mut(&mk).unwrap().push(elem);
    }

    let map: std::collections::HashMap<MapKey, Value> = order
        .into_iter()
        .map(|k| {
            let v = Value::Array(groups.remove(&k).unwrap());
            (k, v)
        })
        .collect();

    Ok(Value::Map(map))
}

/// `array_partition(arr, fn) -> [[passing], [failing]]`
///
/// Splits `arr` into two sub-arrays based on a boolean predicate `fn`. The
/// result is a 2-element Array: the first element is the Array of elements
/// for which `fn` returned true, the second is those for which it returned
/// false. Relative order within each partition is preserved.
///
/// ```text
/// let parts = array_partition([1,2,3,4,5], fn(int x) -> bool { x % 2 == 0 });
/// // parts[0] == [2, 4]   (evens)
/// // parts[1] == [1, 3, 5] (odds)
/// ```
pub(crate) fn builtin_array_partition(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (arr, f) = match args {
        [Value::Array(a), f] => (a.clone(), f.clone()),
        [a, _] => {
            return Err(format!(
                "array_partition: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_partition: expected 2 arguments (array, fn), got {}",
                args.len()
            ));
        }
    };

    let mut passing: Vec<Value> = Vec::new();
    let mut failing: Vec<Value> = Vec::new();
    for elem in arr {
        let pred_val = interp.apply_function(&f, vec![elem.clone()])?;
        match pred_val {
            Value::Bool(true) => passing.push(elem),
            Value::Bool(false) => failing.push(elem),
            other => {
                return Err(format!(
                    "array_partition: predicate must return bool, got {other}"
                ));
            }
        }
    }
    Ok(Value::Array(vec![
        Value::Array(passing),
        Value::Array(failing),
    ]))
}

/// `map_from_pairs(pairs) -> Map`
///
/// Constructs a Map from an Array of 2-element Arrays `[[key, value], ...]`.
/// Each inner Array must have exactly 2 elements; the key must be hashable
/// (int, string, bool). If duplicate keys appear, later entries win.
///
/// ```text
/// let m = map_from_pairs([["a", 1], ["b", 2]]);
/// // m == {"a" -> 1, "b" -> 2}
/// ```
pub(crate) fn builtin_map_from_pairs(args: &[Value]) -> RResult<Value> {
    let pairs = match args {
        [Value::Array(a)] => a.clone(),
        [other] => {
            return Err(format!(
                "map_from_pairs: expected an Array of pairs, got {other}"
            ));
        }
        _ => {
            return Err(format!(
                "map_from_pairs: expected 1 argument, got {}",
                args.len()
            ));
        }
    };

    let mut map: std::collections::HashMap<MapKey, Value> =
        std::collections::HashMap::with_capacity(pairs.len());

    for (i, pair) in pairs.into_iter().enumerate() {
        match pair {
            Value::Array(ref kv) if kv.len() == 2 => {
                let mk = MapKey::from_value(&kv[0])
                    .map_err(|e| format!("map_from_pairs: pair[{i}] key is not hashable: {e}"))?;
                map.insert(mk, kv[1].clone());
            }
            Value::Array(ref kv) => {
                return Err(format!(
                    "map_from_pairs: pair[{i}] must have exactly 2 elements, got {}",
                    kv.len()
                ));
            }
            other => {
                return Err(format!(
                    "map_from_pairs: pair[{i}] must be a 2-element Array, got {other}"
                ));
            }
        }
    }

    Ok(Value::Map(map))
}

/// `array_scan(arr, init, fn) -> Array`
///
/// Like `array_reduce` but returns every intermediate accumulator value as an
/// Array, starting with `init`. The result has `len(arr) + 1` elements:
/// `[init, fn(init, arr[0]), fn(prev, arr[1]), ...]`.
///
/// ```text
/// let running_sum = array_scan([1,2,3,4], 0, fn(int acc, int x) -> int { return acc + x; });
/// // running_sum == [0, 1, 3, 6, 10]
/// ```
pub(crate) fn builtin_array_scan(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    let (arr, init, f) = match args {
        [Value::Array(a), init, f] => (a.clone(), init.clone(), f.clone()),
        [a, _, _] => {
            return Err(format!(
                "array_scan: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_scan: expected 3 arguments (array, init, fn), got {}",
                args.len()
            ));
        }
    };

    let mut out = Vec::with_capacity(arr.len() + 1);
    let mut acc = init;
    out.push(acc.clone());
    for elem in arr {
        acc = interp.apply_function(&f, vec![acc, elem])?;
        out.push(acc.clone());
    }
    Ok(Value::Array(out))
}

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── array_flat_map ────────────────────────────────────────────────────────

    #[test]
    fn flat_map_basic() {
        // [1,2,3] × fn(x) -> [x, x*10] → [1,10, 2,20, 3,30]
        let r = run(
            r#"let result = array_flat_map([1, 2, 3], fn(int x) -> Array { [x, x * 10] });
println(result[0]);
println(result[1]);
println(result[3]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('1'), "first: {}", r.stdout);
        assert!(r.stdout.contains("10"), "second: {}", r.stdout);
        assert!(r.stdout.contains("20"), "fourth (index 3): {}", r.stdout);
    }

    #[test]
    fn flat_map_empty_input() {
        let r = run(
            r#"let result = array_flat_map([], fn(int x) -> Array { [x] });
println(len(result));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "len: {}", r.stdout);
    }

    #[test]
    fn flat_map_empty_inner_arrays() {
        let r = run(
            r#"let result = array_flat_map([1, 2, 3], fn(int x) -> Array { [] });
println(len(result));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "len: {}", r.stdout);
    }

    #[test]
    fn flat_map_rejects_non_array_callback_return() {
        let r = run(
            r#"let result = array_flat_map([1, 2], fn(int x) -> int { x });
println(result);"#,
        );
        assert!(!r.ok, "expected error for non-array callback return");
    }

    // ── array_group_by ────────────────────────────────────────────────────────

    #[test]
    fn group_by_even_odd() {
        let r = run(
            r#"let m = array_group_by([1, 2, 3, 4], fn(int x) -> string {
    if x % 2 == 0 { return "even"; }
    return "odd";
});
println(len(m["even"]));
println(len(m["odd"]));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('2'), "even count: {}", r.stdout);
    }

    #[test]
    fn group_by_empty_input() {
        let r = run(r#"let m = array_group_by([], fn(int x) -> string { "k" });
println(map_len(m));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "len: {}", r.stdout);
    }

    #[test]
    fn group_by_rejects_non_hashable_key() {
        let r = run(
            r#"let m = array_group_by([1.0, 2.0], fn(float x) -> float { x });
println(m);"#,
        );
        assert!(!r.ok, "expected error for non-hashable key");
    }

    // ── array_partition ───────────────────────────────────────────────────────

    #[test]
    fn partition_even_odd() {
        let r = run(
            r#"let parts = array_partition([1, 2, 3, 4, 5], fn(int x) -> bool { return x % 2 == 0; });
println(len(parts[0]));
println(len(parts[1]));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "2", "even count: {}", r.stdout);
        assert_eq!(lines[1], "3", "odd count: {}", r.stdout);
    }

    #[test]
    fn partition_all_pass() {
        let r = run(
            r#"let parts = array_partition([2, 4, 6], fn(int x) -> bool { return x % 2 == 0; });
println(len(parts[0]));
println(len(parts[1]));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "3", "passing: {}", r.stdout);
        assert_eq!(lines[1], "0", "failing: {}", r.stdout);
    }

    #[test]
    fn partition_empty_input() {
        let r = run(
            r#"let parts = array_partition([], fn(int x) -> bool { return true; });
println(len(parts[0]));
println(len(parts[1]));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "0", "passing: {}", r.stdout);
        assert_eq!(lines[1], "0", "failing: {}", r.stdout);
    }

    // ── map_from_pairs ────────────────────────────────────────────────────────

    #[test]
    fn map_from_pairs_basic() {
        let r = run(r#"let m = map_from_pairs([["a", 1], ["b", 2]]);
println(m["a"]);
println(m["b"]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('1'), "a: {}", r.stdout);
        assert!(r.stdout.contains('2'), "b: {}", r.stdout);
    }

    #[test]
    fn map_from_pairs_int_keys() {
        let r = run(r#"let m = map_from_pairs([[0, "zero"], [1, "one"]]);
println(m[0]);
println(m[1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("zero"), "0: {}", r.stdout);
        assert!(r.stdout.contains("one"), "1: {}", r.stdout);
    }

    #[test]
    fn map_from_pairs_empty() {
        let r = run(r#"let m = map_from_pairs([]);
println(map_len(m));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "len: {}", r.stdout);
    }

    #[test]
    fn map_from_pairs_rejects_non_pair() {
        let r = run(r#"let m = map_from_pairs([[1, 2, 3]]);
println(m);"#);
        assert!(!r.ok, "expected error for 3-element pair");
    }

    #[test]
    fn map_from_pairs_duplicate_keys_last_wins() {
        let r = run(r#"let m = map_from_pairs([["x", 1], ["x", 99]]);
println(m["x"]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("99"), "last wins: {}", r.stdout);
    }
}
