//! RES-1146: float / string array sort + the `array_is_sorted` predicate
//! family.
//!
//! `array_sort` and `array_sort_desc` only accept int arrays; this
//! ticket adds the float / string variants and the "is this already
//! sorted?" predicate over all three element types.
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `array_sort_float(arr)`      | `(Array) -> Array` | Ascending float sort (NaN-safe via IEEE 754 total order) |
//! | `array_sort_string(arr)`     | `(Array) -> Array` | Ascending lex string sort |
//! | `array_is_sorted(arr)`       | `(Array) -> Bool`  | Int array is ascending |
//! | `array_is_sorted_float(arr)` | `(Array) -> Bool`  | Float array is ascending (NaN-safe) |
//! | `array_is_sorted_string(arr)`| `(Array) -> Bool`  | String array is lex-ascending |
//!
//! All five are pure leaf builtins. `_float` variants use
//! `f64::total_cmp` so NaN and `±0` are well-ordered. Empty and
//! singleton arrays are always sorted (vacuously).

use crate::{RResult, Value};

fn collect_floats(name: &str, items: &[Value]) -> RResult<Vec<f64>> {
    let mut nums: Vec<f64> = Vec::with_capacity(items.len());
    for v in items {
        match v {
            Value::Float(f) => nums.push(*f),
            other => {
                return Err(format!(
                    "{}: expected all float elements, got {}",
                    name, other
                ));
            }
        }
    }
    Ok(nums)
}

fn collect_strings(name: &str, items: &[Value]) -> RResult<Vec<String>> {
    let mut out: Vec<String> = Vec::with_capacity(items.len());
    for v in items {
        match v {
            Value::String(s) => out.push(s.clone()),
            other => {
                return Err(format!(
                    "{}: expected all string elements, got {}",
                    name, other
                ));
            }
        }
    }
    Ok(out)
}

// RES-2032: `collect_ints` was previously used by `is_sorted` to
// materialize the entire input into a `Vec<i64>` before checking
// sortedness. The new single-pass `is_sorted` walks `&[Value]`
// directly with a `prev` register, so the helper is no longer needed.
// `collect_floats` / `collect_strings` remain because `sort_float` /
// `sort_string` need random access on a typed Vec for sorting.

/// `array_sort_float(arr) -> Array` — sort a float array ascending using
/// IEEE 754 total order (`f64::total_cmp`). NaN, `±0`, and signed-NaN
/// payloads are placed in a well-defined position; the result is always
/// a strict total order, unlike `<` which is undefined on NaN.
pub(crate) fn builtin_array_sort_float(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            let mut nums = collect_floats("array_sort_float", items)?;
            nums.sort_by(|a, b| a.total_cmp(b));
            Ok(Value::Array(nums.into_iter().map(Value::Float).collect()))
        }
        [other] => Err(format!("array_sort_float: expected array, got {}", other)),
        _ => Err(format!(
            "array_sort_float: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_sort_string(arr) -> Array` — sort a string array ascending
/// lexicographically (byte-wise on the UTF-8 representation, matching
/// `String::cmp`).
pub(crate) fn builtin_array_sort_string(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            let mut strings = collect_strings("array_sort_string", items)?;
            strings.sort();
            Ok(Value::Array(
                strings.into_iter().map(Value::String).collect(),
            ))
        }
        [other] => Err(format!("array_sort_string: expected array, got {}", other)),
        _ => Err(format!(
            "array_sort_string: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_is_sorted(arr) -> Bool` — true iff every consecutive pair of
/// int elements is non-decreasing. Empty and singleton arrays are
/// vacuously sorted.
///
/// RES-2032: single-pass scan over `&[Value]`. The previous
/// implementation materialized a full `Vec<i64>` via `collect_ints` and
/// then walked `windows(2).all(...)` — two passes plus one throwaway
/// allocation. The new form validates types inline and returns
/// `Bool(false)` on the first inversion.
pub(crate) fn builtin_array_is_sorted(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            let mut iter = items.iter();
            let mut prev = match iter.next() {
                None => return Ok(Value::Bool(true)),
                Some(Value::Int(n)) => *n,
                Some(other) => {
                    return Err(format!(
                        "array_is_sorted: expected all int elements, got {}",
                        other
                    ));
                }
            };
            for v in iter {
                let curr = match v {
                    Value::Int(n) => *n,
                    other => {
                        return Err(format!(
                            "array_is_sorted: expected all int elements, got {}",
                            other
                        ));
                    }
                };
                if curr < prev {
                    return Ok(Value::Bool(false));
                }
                prev = curr;
            }
            Ok(Value::Bool(true))
        }
        [other] => Err(format!("array_is_sorted: expected array, got {}", other)),
        _ => Err(format!(
            "array_is_sorted: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_is_sorted_float(arr) -> Bool` — true iff every consecutive
/// pair is `<=` under IEEE 754 total order. Same NaN-safety as
/// `array_sort_float`.
///
/// RES-2032: single-pass form. See `array_is_sorted` for the rationale.
pub(crate) fn builtin_array_is_sorted_float(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            let mut iter = items.iter();
            let mut prev = match iter.next() {
                None => return Ok(Value::Bool(true)),
                Some(Value::Float(f)) => *f,
                Some(other) => {
                    return Err(format!(
                        "array_is_sorted_float: expected all float elements, got {}",
                        other
                    ));
                }
            };
            for v in iter {
                let curr = match v {
                    Value::Float(f) => *f,
                    other => {
                        return Err(format!(
                            "array_is_sorted_float: expected all float elements, got {}",
                            other
                        ));
                    }
                };
                if curr.total_cmp(&prev) == std::cmp::Ordering::Less {
                    return Ok(Value::Bool(false));
                }
                prev = curr;
            }
            Ok(Value::Bool(true))
        }
        [other] => Err(format!(
            "array_is_sorted_float: expected array, got {}",
            other
        )),
        _ => Err(format!(
            "array_is_sorted_float: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_is_sorted_string(arr) -> Bool` — true iff every consecutive
/// pair is lex-ordered.
///
/// RES-2032: single-pass form. The previous `collect_strings` helper
/// cloned every string element into a `Vec<String>` before comparing;
/// the new form holds `prev: &str` borrowed directly into the input,
/// so no string clones happen. Big win for long arrays of long strings.
pub(crate) fn builtin_array_is_sorted_string(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            let mut iter = items.iter();
            let mut prev: &str = match iter.next() {
                None => return Ok(Value::Bool(true)),
                Some(Value::String(s)) => s.as_str(),
                Some(other) => {
                    return Err(format!(
                        "array_is_sorted_string: expected all string elements, got {}",
                        other
                    ));
                }
            };
            for v in iter {
                let curr: &str = match v {
                    Value::String(s) => s.as_str(),
                    other => {
                        return Err(format!(
                            "array_is_sorted_string: expected all string elements, got {}",
                            other
                        ));
                    }
                };
                if curr < prev {
                    return Ok(Value::Bool(false));
                }
                prev = curr;
            }
            Ok(Value::Bool(true))
        }
        [other] => Err(format!(
            "array_is_sorted_string: expected array, got {}",
            other
        )),
        _ => Err(format!(
            "array_is_sorted_string: expected 1 argument, got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floats(xs: &[f64]) -> Value {
        Value::Array(xs.iter().map(|&f| Value::Float(f)).collect())
    }

    fn strings(xs: &[&str]) -> Value {
        Value::Array(xs.iter().map(|s| Value::String(s.to_string())).collect())
    }

    fn ints(xs: &[i64]) -> Value {
        Value::Array(xs.iter().map(|&n| Value::Int(n)).collect())
    }

    fn as_float_vec(v: Value) -> Vec<f64> {
        match v {
            Value::Array(items) => items
                .into_iter()
                .map(|x| match x {
                    Value::Float(f) => f,
                    other => panic!("expected Float, got {:?}", other),
                })
                .collect(),
            other => panic!("expected Array, got {:?}", other),
        }
    }

    fn as_string_vec(v: Value) -> Vec<String> {
        match v {
            Value::Array(items) => items
                .into_iter()
                .map(|x| match x {
                    Value::String(s) => s,
                    other => panic!("expected String, got {:?}", other),
                })
                .collect(),
            other => panic!("expected Array, got {:?}", other),
        }
    }

    fn as_bool(v: Value) -> bool {
        match v {
            Value::Bool(b) => b,
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    // --- sort_float -------------------------------------------------------

    #[test]
    fn sort_float_basic() {
        let r = builtin_array_sort_float(&[floats(&[3.0, 1.5, 2.5, -1.0])]).unwrap();
        assert_eq!(as_float_vec(r), vec![-1.0, 1.5, 2.5, 3.0]);
    }

    #[test]
    fn sort_float_empty() {
        let r = builtin_array_sort_float(&[floats(&[])]).unwrap();
        assert_eq!(as_float_vec(r), Vec::<f64>::new());
    }

    #[test]
    fn sort_float_handles_nan_under_total_order() {
        // NaN sorts after +inf under IEEE 754 total order.
        let r = builtin_array_sort_float(&[floats(&[1.0, f64::NAN, -1.0, f64::INFINITY, 0.0])])
            .unwrap();
        let v = as_float_vec(r);
        assert_eq!(v.len(), 5);
        assert_eq!(v[0], -1.0);
        assert_eq!(v[1], 0.0);
        assert_eq!(v[2], 1.0);
        assert_eq!(v[3], f64::INFINITY);
        // Last is NaN — can't use ==, check classify.
        assert!(v[4].is_nan());
    }

    #[test]
    fn sort_float_distinguishes_signed_zero() {
        // -0.0 < +0.0 under total order.
        let r = builtin_array_sort_float(&[floats(&[0.0, -0.0, 0.0])]).unwrap();
        let v = as_float_vec(r);
        assert!(v[0].is_sign_negative());
        assert!(!v[1].is_sign_negative());
        assert!(!v[2].is_sign_negative());
    }

    #[test]
    fn sort_float_rejects_non_float() {
        let err = builtin_array_sort_float(&[ints(&[1, 2, 3])]).unwrap_err();
        assert!(err.contains("expected all float elements"));
    }

    #[test]
    fn sort_float_rejects_non_array() {
        let err = builtin_array_sort_float(&[Value::Int(7)]).unwrap_err();
        assert!(err.contains("expected array"));
    }

    // --- sort_string ------------------------------------------------------

    #[test]
    fn sort_string_basic() {
        let r = builtin_array_sort_string(&[strings(&["carol", "alice", "bob"])]).unwrap();
        assert_eq!(as_string_vec(r), vec!["alice", "bob", "carol"]);
    }

    #[test]
    fn sort_string_empty() {
        let r = builtin_array_sort_string(&[strings(&[])]).unwrap();
        assert_eq!(as_string_vec(r), Vec::<String>::new());
    }

    #[test]
    fn sort_string_lex_byte_order() {
        // Uppercase < lowercase under byte-wise UTF-8 comparison.
        let r = builtin_array_sort_string(&[strings(&["b", "A", "a", "B"])]).unwrap();
        assert_eq!(as_string_vec(r), vec!["A", "B", "a", "b"]);
    }

    #[test]
    fn sort_string_stable_on_duplicates() {
        let r = builtin_array_sort_string(&[strings(&["x", "a", "x", "a"])]).unwrap();
        assert_eq!(as_string_vec(r), vec!["a", "a", "x", "x"]);
    }

    #[test]
    fn sort_string_rejects_non_string() {
        let err = builtin_array_sort_string(&[floats(&[1.0])]).unwrap_err();
        assert!(err.contains("expected all string elements"));
    }

    // --- is_sorted (int) --------------------------------------------------

    #[test]
    fn is_sorted_int_basic() {
        assert!(as_bool(
            builtin_array_is_sorted(&[ints(&[1, 2, 3, 4, 5])]).unwrap()
        ));
        assert!(!as_bool(
            builtin_array_is_sorted(&[ints(&[1, 3, 2])]).unwrap()
        ));
        // Equal-adjacent counts as sorted.
        assert!(as_bool(
            builtin_array_is_sorted(&[ints(&[1, 1, 1])]).unwrap()
        ));
    }

    #[test]
    fn is_sorted_int_vacuous() {
        assert!(as_bool(builtin_array_is_sorted(&[ints(&[])]).unwrap()));
        assert!(as_bool(builtin_array_is_sorted(&[ints(&[42])]).unwrap()));
    }

    #[test]
    fn is_sorted_int_rejects_non_int_elements() {
        let err = builtin_array_is_sorted(&[Value::Array(vec![Value::Int(1), Value::Float(2.0)])])
            .unwrap_err();
        assert!(err.contains("expected all int elements"));
    }

    // --- is_sorted_float --------------------------------------------------

    #[test]
    fn is_sorted_float_basic() {
        assert!(as_bool(
            builtin_array_is_sorted_float(&[floats(&[-1.0, 0.0, 1.5, 2.5])]).unwrap()
        ));
        assert!(!as_bool(
            builtin_array_is_sorted_float(&[floats(&[1.0, 0.5])]).unwrap()
        ));
    }

    #[test]
    fn is_sorted_float_nan_sorts_last_under_total_order() {
        // Under total order, NaN > +inf — so [-inf, 0, NaN] is sorted.
        assert!(as_bool(
            builtin_array_is_sorted_float(&[floats(&[
                f64::NEG_INFINITY,
                0.0,
                f64::INFINITY,
                f64::NAN
            ])])
            .unwrap()
        ));
        // [NaN, 0] is NOT sorted (NaN > 0).
        assert!(!as_bool(
            builtin_array_is_sorted_float(&[floats(&[f64::NAN, 0.0])]).unwrap()
        ));
    }

    #[test]
    fn is_sorted_float_signed_zero() {
        // -0.0 < +0.0 under total order, so [-0.0, +0.0] IS sorted.
        assert!(as_bool(
            builtin_array_is_sorted_float(&[floats(&[-0.0, 0.0])]).unwrap()
        ));
        // [+0.0, -0.0] is NOT sorted.
        assert!(!as_bool(
            builtin_array_is_sorted_float(&[floats(&[0.0, -0.0])]).unwrap()
        ));
    }

    // --- is_sorted_string -------------------------------------------------

    #[test]
    fn is_sorted_string_basic() {
        assert!(as_bool(
            builtin_array_is_sorted_string(&[strings(&["a", "b", "c"])]).unwrap()
        ));
        assert!(!as_bool(
            builtin_array_is_sorted_string(&[strings(&["c", "b", "a"])]).unwrap()
        ));
    }

    #[test]
    fn is_sorted_string_vacuous() {
        assert!(as_bool(
            builtin_array_is_sorted_string(&[strings(&[])]).unwrap()
        ));
        assert!(as_bool(
            builtin_array_is_sorted_string(&[strings(&["only"])]).unwrap()
        ));
    }

    // --- composite property -----------------------------------------------

    #[test]
    fn sort_produces_sorted_output() {
        // Sorting any input should produce something is_sorted accepts.
        let inputs = [floats(&[3.0, 1.0, 2.0, 0.5, -1.0])];
        for input in inputs {
            let sorted = builtin_array_sort_float(&[input]).unwrap();
            assert!(as_bool(builtin_array_is_sorted_float(&[sorted]).unwrap()));
        }

        let inputs = [strings(&["banana", "apple", "cherry"])];
        for input in inputs {
            let sorted = builtin_array_sort_string(&[input]).unwrap();
            assert!(as_bool(builtin_array_is_sorted_string(&[sorted]).unwrap()));
        }
    }

    #[test]
    fn arity_diagnostics_consistent() {
        for f in [
            builtin_array_sort_float,
            builtin_array_sort_string,
            builtin_array_is_sorted,
            builtin_array_is_sorted_float,
            builtin_array_is_sorted_string,
        ] {
            let err = f(&[]).unwrap_err();
            assert!(err.contains("expected 1"), "got {}", err);
            let err = f(&[Value::Int(5)]).unwrap_err();
            assert!(err.contains("expected array"), "got {}", err);
        }
    }
}
