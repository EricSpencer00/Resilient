//! RES-1160: argmax / argmin for float and string arrays.
//!
//! Complements the existing `array_argmax_int` / `array_argmin_int`
//! (RES-???). Argmin / argmax returns the index of the (first)
//! min/max element — the standard primitive for peak detection,
//! feature ranking, and tournament selection.
//!
//! | Builtin | Signature | Notes |
//! |---|---|---|
//! | `array_argmax_float(arr)`  | `(Array) -> Int` | NaN-safe via IEEE 754 total order |
//! | `array_argmin_float(arr)`  | `(Array) -> Int` | Same |
//! | `array_argmax_string(arr)` | `(Array) -> Int` | Lex byte-wise |
//! | `array_argmin_string(arr)` | `(Array) -> Int` | Same |
//!
//! Empty array → typed error. First match wins on ties.

use crate::{RResult, Value};

// RES-2026: single-pass scan helpers. The previous `collect_floats` /
// `collect_strings` helpers materialized an intermediate `Vec<f64>` /
// `Vec<&String>` from the entire input array, then a second loop
// walked that Vec to find the best index. The intermediate is
// throwaway — we only need the index. Inlining the type-check into
// the scan loop drops the allocation entirely.

/// Scan `items` left-to-right, tracking the index whose extracted
/// `f64` is "best" by `is_better(candidate, current_best)`. Returns
/// the index of the first such element. Empty array errors via
/// `name`. Type-mismatch errors carry the element that failed.
fn argbest_float(name: &str, items: &[Value], is_better: fn(f64, f64) -> bool) -> RResult<Value> {
    if items.is_empty() {
        return Err(format!("{}: empty array has no argmax/argmin", name));
    }
    let first = match &items[0] {
        Value::Float(f) => *f,
        other => {
            return Err(format!(
                "{}: expected all float elements, got {}",
                name, other
            ));
        }
    };
    let mut best_idx = 0usize;
    let mut best_val = first;
    for (i, v) in items.iter().enumerate().skip(1) {
        let f = match v {
            Value::Float(f) => *f,
            other => {
                return Err(format!(
                    "{}: expected all float elements, got {}",
                    name, other
                ));
            }
        };
        if is_better(f, best_val) {
            best_val = f;
            best_idx = i;
        }
    }
    Ok(Value::Int(best_idx as i64))
}

/// String variant of `argbest_float`. Comparator takes `&str` slices.
fn argbest_string(
    name: &str,
    items: &[Value],
    is_better: fn(&str, &str) -> bool,
) -> RResult<Value> {
    if items.is_empty() {
        return Err(format!("{}: empty array has no argmax/argmin", name));
    }
    let first = match &items[0] {
        Value::String(s) => s.as_str(),
        other => {
            return Err(format!(
                "{}: expected all string elements, got {}",
                name, other
            ));
        }
    };
    let mut best_idx = 0usize;
    let mut best_val: &str = first;
    for (i, v) in items.iter().enumerate().skip(1) {
        let s = match v {
            Value::String(s) => s.as_str(),
            other => {
                return Err(format!(
                    "{}: expected all string elements, got {}",
                    name, other
                ));
            }
        };
        if is_better(s, best_val) {
            best_val = s;
            best_idx = i;
        }
    }
    Ok(Value::Int(best_idx as i64))
}

/// `array_argmax_float(arr) -> Int` — index of the maximum element
/// under IEEE 754 total order. First match wins on ties.
pub(crate) fn builtin_array_argmax_float(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => argbest_float("array_argmax_float", items, |c, b| {
            c.total_cmp(&b) == std::cmp::Ordering::Greater
        }),
        [other] => Err(format!("array_argmax_float: expected array, got {}", other)),
        _ => Err(format!(
            "array_argmax_float: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_argmin_float(arr) -> Int` — index of the minimum element
/// under IEEE 754 total order.
pub(crate) fn builtin_array_argmin_float(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => argbest_float("array_argmin_float", items, |c, b| {
            c.total_cmp(&b) == std::cmp::Ordering::Less
        }),
        [other] => Err(format!("array_argmin_float: expected array, got {}", other)),
        _ => Err(format!(
            "array_argmin_float: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_argmax_string(arr) -> Int` — index of the lex-maximum string.
pub(crate) fn builtin_array_argmax_string(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => argbest_string("array_argmax_string", items, |c, b| c > b),
        [other] => Err(format!(
            "array_argmax_string: expected array, got {}",
            other
        )),
        _ => Err(format!(
            "array_argmax_string: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_argmin_string(arr) -> Int` — index of the lex-minimum string.
pub(crate) fn builtin_array_argmin_string(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => argbest_string("array_argmin_string", items, |c, b| c < b),
        [other] => Err(format!(
            "array_argmin_string: expected array, got {}",
            other
        )),
        _ => Err(format!(
            "array_argmin_string: expected 1 argument, got {}",
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

    fn as_int(v: Value) -> i64 {
        match v {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    // --- argmax_float ---

    #[test]
    fn argmax_float_basic() {
        let r = builtin_array_argmax_float(&[floats(&[1.0, 3.0, 2.0, 5.0, 4.0])]).unwrap();
        assert_eq!(as_int(r), 3);
    }

    #[test]
    fn argmax_float_single_element_is_zero() {
        let r = builtin_array_argmax_float(&[floats(&[42.0])]).unwrap();
        assert_eq!(as_int(r), 0);
    }

    #[test]
    fn argmax_float_ties_pick_first() {
        let r = builtin_array_argmax_float(&[floats(&[3.0, 1.0, 3.0, 2.0])]).unwrap();
        assert_eq!(as_int(r), 0);
    }

    #[test]
    fn argmax_float_handles_nan_under_total_order() {
        // NaN is the maximum under IEEE 754 total order.
        let r = builtin_array_argmax_float(&[floats(&[1.0, f64::NAN, 100.0])]).unwrap();
        assert_eq!(as_int(r), 1);
    }

    #[test]
    fn argmax_float_signed_zero() {
        // +0 > -0 under total order, so [+0, -0] argmax is 0.
        let r = builtin_array_argmax_float(&[floats(&[0.0, -0.0])]).unwrap();
        assert_eq!(as_int(r), 0);
        // [-0, +0] argmax is 1.
        let r = builtin_array_argmax_float(&[floats(&[-0.0, 0.0])]).unwrap();
        assert_eq!(as_int(r), 1);
    }

    #[test]
    fn argmax_float_empty_errors() {
        let err = builtin_array_argmax_float(&[floats(&[])]).unwrap_err();
        assert!(err.contains("empty array"));
    }

    // --- argmin_float ---

    #[test]
    fn argmin_float_basic() {
        let r = builtin_array_argmin_float(&[floats(&[3.0, 1.0, 5.0, 2.0])]).unwrap();
        assert_eq!(as_int(r), 1);
    }

    #[test]
    fn argmin_float_ties_pick_first() {
        let r = builtin_array_argmin_float(&[floats(&[1.0, 3.0, 1.0])]).unwrap();
        assert_eq!(as_int(r), 0);
    }

    #[test]
    fn argmin_float_handles_neg_inf() {
        // -inf is minimum.
        let r =
            builtin_array_argmin_float(&[floats(&[0.0, -1.0, f64::NEG_INFINITY, 1.0])]).unwrap();
        assert_eq!(as_int(r), 2);
    }

    #[test]
    fn argmin_float_empty_errors() {
        let err = builtin_array_argmin_float(&[floats(&[])]).unwrap_err();
        assert!(err.contains("empty array"));
    }

    // --- argmax_string ---

    #[test]
    fn argmax_string_basic() {
        let r = builtin_array_argmax_string(&[strings(&["a", "c", "b"])]).unwrap();
        assert_eq!(as_int(r), 1);
    }

    #[test]
    fn argmax_string_lex_byte_order() {
        // Uppercase < lowercase in byte order, so "a" > "Z".
        let r = builtin_array_argmax_string(&[strings(&["Z", "a", "B"])]).unwrap();
        assert_eq!(as_int(r), 1);
    }

    #[test]
    fn argmax_string_ties_pick_first() {
        let r = builtin_array_argmax_string(&[strings(&["b", "a", "b"])]).unwrap();
        assert_eq!(as_int(r), 0);
    }

    // --- argmin_string ---

    #[test]
    fn argmin_string_basic() {
        let r = builtin_array_argmin_string(&[strings(&["carol", "alice", "bob"])]).unwrap();
        assert_eq!(as_int(r), 1);
    }

    #[test]
    fn argmin_string_empty_errors() {
        let err = builtin_array_argmin_string(&[strings(&[])]).unwrap_err();
        assert!(err.contains("empty array"));
    }

    // --- general ---

    #[test]
    fn arity_diagnostics_consistent() {
        for f in [
            builtin_array_argmax_float,
            builtin_array_argmin_float,
            builtin_array_argmax_string,
            builtin_array_argmin_string,
        ] {
            let err = f(&[]).unwrap_err();
            assert!(err.contains("expected 1"), "got {}", err);
            let err = f(&[Value::Int(5)]).unwrap_err();
            assert!(err.contains("expected array"), "got {}", err);
        }
    }

    #[test]
    fn rejects_mixed_element_types() {
        let err =
            builtin_array_argmax_float(&[Value::Array(vec![Value::Float(1.0), Value::Int(2)])])
                .unwrap_err();
        assert!(err.contains("expected all float elements"));

        let err = builtin_array_argmax_string(&[Value::Array(vec![
            Value::String("a".to_string()),
            Value::Int(0),
        ])])
        .unwrap_err();
        assert!(err.contains("expected all string elements"));
    }

    #[test]
    fn argmax_argmin_complementary_on_distinct_values() {
        // For any array with distinct max and min, argmax and argmin
        // must return different indices.
        for arr in [floats(&[1.0, 5.0, 3.0, 2.0]), floats(&[10.0, 0.0, 5.0])] {
            let max_idx = as_int(builtin_array_argmax_float(std::slice::from_ref(&arr)).unwrap());
            let min_idx = as_int(builtin_array_argmin_float(std::slice::from_ref(&arr)).unwrap());
            assert_ne!(max_idx, min_idx);
        }
    }
}
