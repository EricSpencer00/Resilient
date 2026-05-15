//! RES-2650: Extra collection utilities.
//!
//! * `array_frequency_map(arr)` — count occurrences of each hashable element;
//!   returns a Map<element→count>.
//! * `array_key_by(arr, fn)` — build a Map keyed by `fn(elem)`; if duplicate
//!   keys exist the last element wins.
//! * `array_iterate(init, n, fn)` — apply `fn` to value `n` times, returning
//!   an Array of all `n+1` values starting with `init`.

use crate::{Interpreter, MapKey, Value};

type RResult<T> = Result<T, String>;

/// `array_frequency_map(arr) -> Map`
///
/// Counts how many times each value appears in `arr`. The elements must be
/// hashable (int, string, or bool). Returns a `Map<element → count>`.
///
/// ```text
/// let freq = array_frequency_map(["a","b","a","c","a","b"]);
/// // freq == {"a" -> 3, "b" -> 2, "c" -> 1}
/// ```
pub(crate) fn builtin_array_frequency_map(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arr)] => {
            let mut counts: std::collections::HashMap<MapKey, Value> =
                std::collections::HashMap::new();
            for elem in arr {
                let mk = MapKey::from_value(elem).map_err(|e| {
                    format!(
                        "array_frequency_map: elements must be hashable (int/string/bool): {e}"
                    )
                })?;
                let entry = counts.entry(mk).or_insert(Value::Int(0));
                if let Value::Int(n) = entry {
                    *n += 1;
                }
            }
            Ok(Value::Map(counts))
        }
        [other] => Err(format!(
            "array_frequency_map: expected an Array, got {other}"
        )),
        _ => Err(format!(
            "array_frequency_map: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_key_by(arr, fn) -> Map`
///
/// Builds a Map by applying `fn(elem) -> key` to each element. The key must
/// be hashable (int, string, bool). If multiple elements map to the same key,
/// the last one wins. Useful for O(1) lookup by a computed key.
///
/// ```text
/// let by_name = array_key_by(users, fn(Array u) -> string { return u[0]; });
/// // if users == [["alice", 30], ["bob", 25]]
/// // by_name == {"alice" -> ["alice",30], "bob" -> ["bob",25]}
/// ```
pub(crate) fn builtin_array_key_by(
    interp: &mut Interpreter,
    args: &[Value],
) -> RResult<Value> {
    let (arr, f) = match args {
        [Value::Array(a), f] => (a.clone(), f.clone()),
        [a, _] => {
            return Err(format!(
                "array_key_by: first argument must be an Array, got {a}"
            ))
        }
        _ => {
            return Err(format!(
                "array_key_by: expected 2 arguments (array, fn), got {}",
                args.len()
            ))
        }
    };

    let mut map: std::collections::HashMap<MapKey, Value> =
        std::collections::HashMap::with_capacity(arr.len());

    for elem in arr {
        let key_val = interp.apply_function(f.clone(), vec![elem.clone()])?;
        let mk = MapKey::from_value(&key_val).map_err(|e| {
            format!("array_key_by: key function returned non-hashable value: {e}")
        })?;
        map.insert(mk, elem);
    }

    Ok(Value::Map(map))
}

/// `array_iterate(init, n, fn) -> Array`
///
/// Builds an array by repeatedly applying `fn` to a value, starting with
/// `init`. Returns `[init, fn(init), fn(fn(init)), ..., fn^n(init)]`
/// (n+1 elements total). `n` must be >= 0.
///
/// ```text
/// let powers = array_iterate(1, 5, fn(int x) -> int { return x * 2; });
/// // powers == [1, 2, 4, 8, 16, 32]
/// ```
pub(crate) fn builtin_array_iterate(
    interp: &mut Interpreter,
    args: &[Value],
) -> RResult<Value> {
    let (init, n, f) = match args {
        [init, Value::Int(n), f] => (init.clone(), *n, f.clone()),
        [_, n, _] => {
            return Err(format!(
                "array_iterate: second argument must be an int, got {n}"
            ))
        }
        _ => {
            return Err(format!(
                "array_iterate: expected 3 arguments (init, n, fn), got {}",
                args.len()
            ))
        }
    };

    if n < 0 {
        return Err(format!(
            "array_iterate: n must be >= 0, got {n}"
        ));
    }

    let mut out = Vec::with_capacity((n as usize) + 1);
    let mut current = init;
    out.push(current.clone());
    for _ in 0..n {
        current = interp.apply_function(f.clone(), vec![current])?;
        out.push(current.clone());
    }
    Ok(Value::Array(out))
}

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── array_frequency_map ───────────────────────────────────────────────────

    #[test]
    fn frequency_map_strings() {
        let r = run(
            r#"let freq = array_frequency_map(["a","b","a","c","a","b"]);
println(freq["a"]);
println(freq["b"]);
println(freq["c"]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "3");
        assert_eq!(lines[1], "2");
        assert_eq!(lines[2], "1");
    }

    #[test]
    fn frequency_map_ints() {
        let r = run(
            r#"let freq = array_frequency_map([1,2,2,3,3,3]);
println(freq[3]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "stdout: {}", r.stdout);
    }

    #[test]
    fn frequency_map_empty_input() {
        let r = run(
            r#"let freq = array_frequency_map([]);
println(map_len(freq));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'));
    }

    #[test]
    fn frequency_map_rejects_unhashable() {
        let r = run(r#"let freq = array_frequency_map([1.5, 2.5]);
println(freq);"#);
        assert!(!r.ok, "expected error for float elements");
    }

    // ── array_key_by ──────────────────────────────────────────────────────────

    #[test]
    fn key_by_first_char() {
        let r = run(
            r#"let by_first = array_key_by(["apple","banana","avocado"],
    fn(string s) -> string { return string_at(s, 0); });
println(by_first["b"]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("banana"), "stdout: {}", r.stdout);
    }

    #[test]
    fn key_by_length() {
        let r = run(
            r#"let by_len = array_key_by(["x","xy","xyz"],
    fn(string s) -> int { return len(s); });
println(by_len[3]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("xyz"), "stdout: {}", r.stdout);
    }

    #[test]
    fn key_by_empty_array() {
        let r = run(
            r#"let m = array_key_by([], fn(int x) -> int { return x; });
println(map_len(m));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'));
    }

    // ── array_iterate ─────────────────────────────────────────────────────────

    #[test]
    fn iterate_powers_of_two() {
        let r = run(
            r#"let powers = array_iterate(1, 5, fn(int x) -> int { return x * 2; });
println(len(powers));
println(powers[0]);
println(powers[5]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "6", "6 elements (init + 5 steps)");
        assert_eq!(lines[1], "1");
        assert_eq!(lines[2], "32");
    }

    #[test]
    fn iterate_zero_steps() {
        let r = run(
            r#"let arr = array_iterate(42, 0, fn(int x) -> int { return x + 1; });
println(len(arr));
println(arr[0]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "1", "just init");
        assert_eq!(lines[1], "42");
    }

    #[test]
    fn iterate_negative_n_errors() {
        let r = run(
            r#"let arr = array_iterate(0, -1, fn(int x) -> int { return x; });
println(arr);"#,
        );
        assert!(!r.ok, "expected error for negative n");
    }

    #[test]
    fn iterate_string_accumulator() {
        let r = run(
            r#"let strs = array_iterate("x", 3, fn(string s) -> string { return s + s; });
println(strs[3]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("xxxxxxxx"), "stdout: {}", r.stdout);
    }
}
