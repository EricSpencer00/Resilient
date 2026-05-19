//! RES-2648: Higher-order array combinators with arbitrary callbacks.
//!
//! * `array_sort_by(arr, cmp)` — sort using `cmp(a, b) -> int`
//!   (negative = a before b, 0 = equal, positive = b before a).
//! * `array_min_by(arr, fn)` — return the element for which `fn(elem)` is
//!   smallest (int or float key). Errors on empty array.
//! * `array_max_by(arr, fn)` — return the element for which `fn(elem)` is
//!   largest.
//! * `array_count_if(arr, fn)` — count elements for which `fn(elem)` is true.
//! * `array_zip_with(a, b, fn)` — element-wise combine: `fn(a[i], b[i])`.
//!   Arrays must have the same length.
//! * `array_windows(arr, n)` — overlapping n-element sub-arrays (sliding window).
//! * `array_take_while(arr, fn)` — take elements while `fn(elem)` is true.
//! * `array_drop_while(arr, fn)` — drop elements while `fn(elem)` is true, keep rest.

use crate::{Interpreter, Value};

type RResult<T> = Result<T, String>;

/// `array_sort_by(arr, cmp) -> Array`
///
/// Sorts `arr` using the comparator `cmp(a, b) -> int`. If the return value
/// is negative, `a` comes before `b`; if positive, `b` comes before `a`; if
/// zero they are considered equal. Uses a stable sort so elements comparing
/// equal preserve their original order.
///
/// ```text
/// let sorted = array_sort_by(["banana","apple","cherry"],
///     fn(string a, string b) -> int {
///         if a < b { return -1; }
///         if a > b { return  1; }
///         return 0;
///     });
/// // sorted == ["apple", "banana", "cherry"]
/// ```
pub(crate) fn builtin_array_sort_by(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    // RES-1934: borrow `arr` and `cmp` from `args`. Build the indexed
    // sort buffer in one Vec allocation by cloning straight from
    // `arr.iter()` — the legacy code first cloned `arr` into a fresh
    // Vec, then consumed it into a *second* Vec<(usize, Value)>, doing
    // two Vec allocations where one suffices.
    let (arr, cmp) = match args {
        [Value::Array(a), f] => (a, f),
        [a, _] => {
            return Err(format!(
                "array_sort_by: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_sort_by: expected 2 arguments (array, cmp_fn), got {}",
                args.len()
            ));
        }
    };

    // Collect (index, element) pairs in one allocation, sort by
    // calling the comparator.
    let mut indexed: Vec<(usize, Value)> = arr.iter().cloned().enumerate().collect();
    let mut error: Option<String> = None;

    indexed.sort_by(|(_, a), (_, b)| {
        if error.is_some() {
            return std::cmp::Ordering::Equal;
        }
        match interp.apply_function(cmp, vec![a.clone(), b.clone()]) {
            Ok(Value::Int(n)) => n.cmp(&0),
            Ok(other) => {
                error = Some(format!(
                    "array_sort_by: comparator must return int, got {other}"
                ));
                std::cmp::Ordering::Equal
            }
            Err(e) => {
                error = Some(e);
                std::cmp::Ordering::Equal
            }
        }
    });

    if let Some(e) = error {
        return Err(e);
    }

    Ok(Value::Array(indexed.into_iter().map(|(_, v)| v).collect()))
}

/// `array_min_by(arr, fn) -> Value`
///
/// Returns the element of `arr` for which `fn(elem)` produces the smallest
/// int or float. Errors on an empty array.
///
/// ```text
/// let shortest = array_min_by(["cat","elephant","ox"],
///     fn(string s) -> int { return len(s); });
/// // shortest == "ox"
/// ```
pub(crate) fn builtin_array_min_by(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    // RES-1934: borrow `arr` and `f` from `args`. Iterate `arr.iter()`
    // directly instead of cloning the whole Vec upfront — element
    // clones only happen at the apply_function callsite and when we
    // adopt a new best_elem.
    let (arr, f) = match args {
        [Value::Array(a), f] => (a, f),
        [a, _] => {
            return Err(format!(
                "array_min_by: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_min_by: expected 2 arguments (array, fn), got {}",
                args.len()
            ));
        }
    };

    if arr.is_empty() {
        return Err("array_min_by: cannot find minimum of empty array".to_string());
    }

    let mut best_elem = arr[0].clone();
    let mut best_key = interp.apply_function(f, vec![arr[0].clone()])?;

    for elem in arr.iter().skip(1) {
        let key = interp.apply_function(f, vec![elem.clone()])?;
        let is_smaller = compare_key(&key, &best_key).map_err(|e| format!("array_min_by: {e}"))?;
        if is_smaller < 0 {
            best_elem = elem.clone();
            best_key = key;
        }
    }
    Ok(best_elem)
}

/// `array_max_by(arr, fn) -> Value`
///
/// Returns the element of `arr` for which `fn(elem)` produces the largest
/// int or float. Errors on an empty array.
///
/// ```text
/// let longest = array_max_by(["cat","elephant","ox"],
///     fn(string s) -> int { return len(s); });
/// // longest == "elephant"
/// ```
pub(crate) fn builtin_array_max_by(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    // RES-1934: borrow `arr` and `f`; see `builtin_array_min_by` above.
    let (arr, f) = match args {
        [Value::Array(a), f] => (a, f),
        [a, _] => {
            return Err(format!(
                "array_max_by: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_max_by: expected 2 arguments (array, fn), got {}",
                args.len()
            ));
        }
    };

    if arr.is_empty() {
        return Err("array_max_by: cannot find maximum of empty array".to_string());
    }

    let mut best_elem = arr[0].clone();
    let mut best_key = interp.apply_function(f, vec![arr[0].clone()])?;

    for elem in arr.iter().skip(1) {
        let key = interp.apply_function(f, vec![elem.clone()])?;
        let is_larger = compare_key(&key, &best_key).map_err(|e| format!("array_max_by: {e}"))?;
        if is_larger > 0 {
            best_elem = elem.clone();
            best_key = key;
        }
    }
    Ok(best_elem)
}

/// `array_count_if(arr, fn) -> int`
///
/// Counts how many elements of `arr` satisfy the boolean predicate `fn`.
///
/// ```text
/// let evens = array_count_if([1,2,3,4,5], fn(int x) -> bool { return x % 2 == 0; });
/// // evens == 2
/// ```
pub(crate) fn builtin_array_count_if(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    // RES-2036: borrow `arr` and `f`; clone only per apply_function call.
    let (arr, f) = match args {
        [Value::Array(a), f] => (a, f),
        [a, _] => {
            return Err(format!(
                "array_count_if: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_count_if: expected 2 arguments (array, fn), got {}",
                args.len()
            ));
        }
    };

    let mut count: i64 = 0;
    for elem in arr.iter() {
        match interp.apply_function(f, vec![elem.clone()])? {
            Value::Bool(true) => count += 1,
            Value::Bool(false) => {}
            other => {
                return Err(format!(
                    "array_count_if: predicate must return bool, got {other}"
                ));
            }
        }
    }
    Ok(Value::Int(count))
}

/// `array_zip_with(a, b, fn) -> Array`
///
/// Combines two arrays element-wise using `fn(a[i], b[i]) -> value`. Both
/// arrays must have the same length.
///
/// ```text
/// let sums = array_zip_with([1,2,3], [10,20,30],
///     fn(int a, int b) -> int { return a + b; });
/// // sums == [11, 22, 33]
/// ```
pub(crate) fn builtin_array_zip_with(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    // RES-2036: borrow both arrays and the function; clone only the
    // two values passed to apply_function. Saves *two* full-array
    // clones per call.
    let (a, b, f) = match args {
        [Value::Array(a), Value::Array(b), f] => (a, b, f),
        [Value::Array(_), b, _] => {
            return Err(format!(
                "array_zip_with: second argument must be an Array, got {b}"
            ));
        }
        [a, _, _] => {
            return Err(format!(
                "array_zip_with: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_zip_with: expected 3 arguments (array, array, fn), got {}",
                args.len()
            ));
        }
    };

    if a.len() != b.len() {
        return Err(format!(
            "array_zip_with: arrays must have the same length ({} vs {})",
            a.len(),
            b.len()
        ));
    }

    let mut out = Vec::with_capacity(a.len());
    for (x, y) in a.iter().zip(b.iter()) {
        out.push(interp.apply_function(f, vec![x.clone(), y.clone()])?);
    }
    Ok(Value::Array(out))
}

/// `array_windows(arr, n) -> Array`
///
/// Returns an array of all contiguous sub-arrays of length `n`. For an array
/// of length `L`, produces `max(0, L - n + 1)` windows. `n` must be >= 1.
///
/// ```text
/// let ws = array_windows([1,2,3,4,5], 3);
/// // ws == [[1,2,3], [2,3,4], [3,4,5]]
/// ```
pub(crate) fn builtin_array_windows(args: &[Value]) -> RResult<Value> {
    // RES-2036: borrow `arr`; only per-window `.to_vec()` clones happen.
    let (arr, n) = match args {
        [Value::Array(a), Value::Int(n)] => (a, *n),
        [Value::Array(_), n] => {
            return Err(format!(
                "array_windows: second argument must be an int, got {n}"
            ));
        }
        [a, _] => {
            return Err(format!(
                "array_windows: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_windows: expected 2 arguments (array, n), got {}",
                args.len()
            ));
        }
    };

    if n < 1 {
        return Err(format!("array_windows: window size must be >= 1, got {n}"));
    }
    let n = n as usize;
    if arr.len() < n {
        return Ok(Value::Array(vec![]));
    }
    let mut out = Vec::with_capacity(arr.len() - n + 1);
    for start in 0..=(arr.len() - n) {
        out.push(Value::Array(arr[start..start + n].to_vec()));
    }
    Ok(Value::Array(out))
}

/// `array_take_while(arr, fn) -> Array`
///
/// Returns the longest prefix of `arr` for which `fn(elem)` returns true.
///
/// ```text
/// let prefix = array_take_while([1,2,3,4,1], fn(int x) -> bool { return x < 4; });
/// // prefix == [1, 2, 3]
/// ```
pub(crate) fn builtin_array_take_while(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    // RES-2036: borrow `arr` and `f`; clone only when pushing to `out`
    // or passing to apply_function.
    let (arr, f) = match args {
        [Value::Array(a), f] => (a, f),
        [a, _] => {
            return Err(format!(
                "array_take_while: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_take_while: expected 2 arguments (array, fn), got {}",
                args.len()
            ));
        }
    };

    let mut out = Vec::new();
    for elem in arr.iter() {
        match interp.apply_function(f, vec![elem.clone()])? {
            Value::Bool(true) => out.push(elem.clone()),
            Value::Bool(false) => break,
            other => {
                return Err(format!(
                    "array_take_while: predicate must return bool, got {other}"
                ));
            }
        }
    }
    Ok(Value::Array(out))
}

/// `array_drop_while(arr, fn) -> Array`
///
/// Drops elements from the front of `arr` while `fn(elem)` returns true,
/// returning everything from the first failing element onward.
///
/// ```text
/// let rest = array_drop_while([1,2,3,4,1], fn(int x) -> bool { return x < 3; });
/// // rest == [3, 4, 1]
/// ```
pub(crate) fn builtin_array_drop_while(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    // RES-2036: borrow `arr` and `f`; clone individual elements only
    // when retained (kept after the drop-prefix ends).
    let (arr, f) = match args {
        [Value::Array(a), f] => (a, f),
        [a, _] => {
            return Err(format!(
                "array_drop_while: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_drop_while: expected 2 arguments (array, fn), got {}",
                args.len()
            ));
        }
    };

    let mut dropping = true;
    let mut out = Vec::new();
    for elem in arr.iter() {
        if dropping {
            match interp.apply_function(f, vec![elem.clone()])? {
                Value::Bool(true) => continue,
                Value::Bool(false) => {
                    dropping = false;
                    out.push(elem.clone());
                }
                other => {
                    return Err(format!(
                        "array_drop_while: predicate must return bool, got {other}"
                    ));
                }
            }
        } else {
            out.push(elem.clone());
        }
    }
    Ok(Value::Array(out))
}

/// Compare two values for ordering (int, float, or string keys).
/// Returns negative if a < b, 0 if equal, positive if a > b.
/// `array_sum_by(arr, fn) -> int`
///
/// Applies `fn(elem) -> int` to each element and returns the sum of results.
///
/// ```text
/// let total = array_sum_by(["cat","ox","elephant"],
///     fn(string s) -> int { return len(s); });
/// // total == 3 + 2 + 8 == 13
/// ```
pub(crate) fn builtin_array_sum_by(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    // RES-2036: borrow `arr` and `f`; clone only per apply_function call.
    let (arr, f) = match args {
        [Value::Array(a), f] => (a, f),
        [a, _] => {
            return Err(format!(
                "array_sum_by: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_sum_by: expected 2 arguments (array, fn), got {}",
                args.len()
            ));
        }
    };

    let mut total: i64 = 0;
    for elem in arr.iter() {
        match interp.apply_function(f, vec![elem.clone()])? {
            Value::Int(n) => total += n,
            other => {
                return Err(format!(
                    "array_sum_by: callback must return int, got {other}"
                ));
            }
        }
    }
    Ok(Value::Int(total))
}

/// `array_product_by(arr, fn) -> int`
///
/// Applies `fn(elem) -> int` to each element and returns the product of results.
///
/// ```text
/// let prod = array_product_by([1,2,3,4], fn(int x) -> int { return x * 2; });
/// // prod == 2 * 4 * 6 * 8 == 384
/// ```
pub(crate) fn builtin_array_product_by(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    // RES-2036: borrow `arr` and `f`; clone only per apply_function call.
    let (arr, f) = match args {
        [Value::Array(a), f] => (a, f),
        [a, _] => {
            return Err(format!(
                "array_product_by: first argument must be an Array, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "array_product_by: expected 2 arguments (array, fn), got {}",
                args.len()
            ));
        }
    };

    let mut product: i64 = 1;
    for elem in arr.iter() {
        match interp.apply_function(f, vec![elem.clone()])? {
            Value::Int(n) => product *= n,
            other => {
                return Err(format!(
                    "array_product_by: callback must return int, got {other}"
                ));
            }
        }
    }
    Ok(Value::Int(product))
}

fn compare_key(a: &Value, b: &Value) -> RResult<i64> {
    fn ord(o: std::cmp::Ordering) -> i64 {
        match o {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(ord(x.cmp(y))),
        (Value::Float(x), Value::Float(y)) => Ok(x.partial_cmp(y).map(ord).unwrap_or(0)),
        (Value::Int(x), Value::Float(y)) => Ok((*x as f64).partial_cmp(y).map(ord).unwrap_or(0)),
        (Value::Float(x), Value::Int(y)) => Ok(x.partial_cmp(&(*y as f64)).map(ord).unwrap_or(0)),
        (Value::String(a), Value::String(b)) => Ok(ord(a.as_str().cmp(b.as_str()))),
        (other, _) => Err(format!(
            "key function must return int, float, or string — got {other}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── array_sort_by ─────────────────────────────────────────────────────────

    #[test]
    fn sort_by_strings_alphabetically() {
        let r = run(r#"let sorted = array_sort_by(["banana","apple","cherry"],
    fn(string a, string b) -> int {
        if a < b { return -1; }
        if a > b { return 1; }
        return 0;
    });
println(sorted[0]);
println(sorted[1]);
println(sorted[2]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "apple");
        assert_eq!(lines[1], "banana");
        assert_eq!(lines[2], "cherry");
    }

    #[test]
    fn sort_by_ints_descending() {
        let r = run(r#"let sorted = array_sort_by([3,1,4,1,5,9,2,6],
    fn(int a, int b) -> int { return b - a; });
println(sorted[0]);
println(sorted[1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "9");
        assert_eq!(lines[1], "6");
    }

    #[test]
    fn sort_by_empty_array() {
        let r = run(
            r#"let sorted = array_sort_by([], fn(int a, int b) -> int { return a - b; });
println(len(sorted));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'));
    }

    #[test]
    fn sort_by_rejects_non_int_comparator_return() {
        let r = run(
            r#"let sorted = array_sort_by([1,2,3], fn(int a, int b) -> bool { return a < b; });
println(sorted);"#,
        );
        assert!(!r.ok, "expected error for bool comparator return");
    }

    // ── array_min_by / array_max_by ───────────────────────────────────────────

    #[test]
    fn min_by_string_length() {
        let r = run(r#"let shortest = array_min_by(["cat","elephant","ox"],
    fn(string s) -> int { return len(s); });
println(shortest);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("ox"), "stdout: {}", r.stdout);
    }

    #[test]
    fn max_by_string_length() {
        let r = run(r#"let longest = array_max_by(["cat","elephant","ox"],
    fn(string s) -> int { return len(s); });
println(longest);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("elephant"), "stdout: {}", r.stdout);
    }

    #[test]
    fn min_by_empty_errors() {
        let r = run(r#"let m = array_min_by([], fn(int x) -> int { return x; });
println(m);"#);
        assert!(!r.ok, "expected error for empty array");
    }

    #[test]
    fn max_by_int_values() {
        let r = run(r#"let m = array_max_by([3,1,4,1,5,9,2,6],
    fn(int x) -> int { return x; });
println(m);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('9'), "stdout: {}", r.stdout);
    }

    // ── array_count_if ────────────────────────────────────────────────────────

    #[test]
    fn count_if_evens() {
        let r = run(
            r#"let n = array_count_if([1,2,3,4,5,6], fn(int x) -> bool { return x % 2 == 0; });
println(n);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "stdout: {}", r.stdout);
    }

    #[test]
    fn count_if_none_match() {
        let r = run(
            r#"let n = array_count_if([1,3,5], fn(int x) -> bool { return x % 2 == 0; });
println(n);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    #[test]
    fn count_if_empty_array() {
        let r = run(
            r#"let n = array_count_if([], fn(int x) -> bool { return true; });
println(n);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    // ── array_zip_with ────────────────────────────────────────────────────────

    #[test]
    fn zip_with_sum() {
        let r = run(r#"let sums = array_zip_with([1,2,3], [10,20,30],
    fn(int a, int b) -> int { return a + b; });
println(sums[0]);
println(sums[1]);
println(sums[2]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "11");
        assert_eq!(lines[1], "22");
        assert_eq!(lines[2], "33");
    }

    #[test]
    fn zip_with_length_mismatch_errors() {
        let r = run(r#"let r = array_zip_with([1,2], [1],
    fn(int a, int b) -> int { return a + b; });
println(r);"#);
        assert!(!r.ok, "expected error for length mismatch");
    }

    #[test]
    fn zip_with_empty_arrays() {
        let r = run(r#"let r = array_zip_with([], [],
    fn(int a, int b) -> int { return a + b; });
println(len(r));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'));
    }

    // ── array_windows ─────────────────────────────────────────────────────────

    #[test]
    fn windows_basic() {
        let r = run(r#"let ws = array_windows([1,2,3,4,5], 3);
println(len(ws));
println(ws[0][0]);
println(ws[0][2]);
println(ws[2][0]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "3", "3 windows");
        assert_eq!(lines[1], "1", "ws[0][0]");
        assert_eq!(lines[2], "3", "ws[0][2]");
        assert_eq!(lines[3], "3", "ws[2][0]");
    }

    #[test]
    fn windows_size_equals_length() {
        let r = run(r#"let ws = array_windows([1,2,3], 3);
println(len(ws));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('1'), "1 window");
    }

    #[test]
    fn windows_larger_than_array() {
        let r = run(r#"let ws = array_windows([1,2], 5);
println(len(ws));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "0 windows");
    }

    #[test]
    fn windows_invalid_size_errors() {
        let r = run(r#"let ws = array_windows([1,2,3], 0);
println(ws);"#);
        assert!(!r.ok, "expected error for window size 0");
    }

    // ── array_take_while / array_drop_while ───────────────────────────────────

    #[test]
    fn take_while_prefix() {
        let r = run(r#"let prefix = array_take_while([1,2,3,4,1],
    fn(int x) -> bool { return x < 4; });
println(len(prefix));
println(prefix[2]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "3");
        assert_eq!(lines[1], "3");
    }

    #[test]
    fn take_while_empty_result() {
        let r = run(r#"let prefix = array_take_while([5,6,7],
    fn(int x) -> bool { return x < 3; });
println(len(prefix));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'));
    }

    #[test]
    fn take_while_full_array() {
        let r = run(r#"let prefix = array_take_while([1,2,3],
    fn(int x) -> bool { return x > 0; });
println(len(prefix));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'));
    }

    #[test]
    fn drop_while_suffix() {
        let r = run(r#"let rest = array_drop_while([1,2,3,4,1],
    fn(int x) -> bool { return x < 3; });
println(len(rest));
println(rest[0]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "3");
        assert_eq!(lines[1], "3");
    }

    #[test]
    fn drop_while_all_pass_returns_empty() {
        let r = run(r#"let rest = array_drop_while([1,2,3],
    fn(int x) -> bool { return x > 0; });
println(len(rest));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'));
    }

    #[test]
    fn drop_while_none_pass_returns_all() {
        let r = run(r#"let rest = array_drop_while([5,6,7],
    fn(int x) -> bool { return x < 3; });
println(len(rest));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'));
    }

    // ── array_sum_by / array_product_by ──────────────────────────────────────

    #[test]
    fn sum_by_string_lengths() {
        let r = run(r#"let total = array_sum_by(["cat","ox","elephant"],
    fn(string s) -> int { return len(s); });
println(total);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("13"), "3+2+8=13: {}", r.stdout);
    }

    #[test]
    fn sum_by_empty_is_zero() {
        let r = run(
            r#"let total = array_sum_by([], fn(int x) -> int { return x; });
println(total);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'));
    }

    #[test]
    fn product_by_doubles() {
        let r = run(
            r#"let prod = array_product_by([1,2,3,4], fn(int x) -> int { return x * 2; });
println(prod);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("384"), "2*4*6*8=384: {}", r.stdout);
    }

    #[test]
    fn product_by_empty_is_one() {
        let r = run(
            r#"let prod = array_product_by([], fn(int x) -> int { return x; });
println(prod);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('1'));
    }
}
