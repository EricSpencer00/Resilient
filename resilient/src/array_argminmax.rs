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

fn collect_floats(name: &str, items: &[Value]) -> RResult<Vec<f64>> {
    if items.is_empty() {
        return Err(format!("{}: empty array has no argmax/argmin", name));
    }
    let mut out: Vec<f64> = Vec::with_capacity(items.len());
    for v in items {
        match v {
            Value::Float(f) => out.push(*f),
            other => {
                return Err(format!(
                    "{}: expected all float elements, got {}",
                    name, other
                ));
            }
        }
    }
    Ok(out)
}

fn collect_strings<'a>(name: &str, items: &'a [Value]) -> RResult<Vec<&'a String>> {
    if items.is_empty() {
        return Err(format!("{}: empty array has no argmax/argmin", name));
    }
    let mut out: Vec<&String> = Vec::with_capacity(items.len());
    for v in items {
        match v {
            Value::String(s) => out.push(s),
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

/// `array_argmax_float(arr) -> Int` — index of the maximum element
/// under IEEE 754 total order. First match wins on ties.
pub(crate) fn builtin_array_argmax_float(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            let nums = collect_floats("array_argmax_float", items)?;
            let mut best_idx = 0usize;
            for (i, &v) in nums.iter().enumerate().skip(1) {
                if v.total_cmp(&nums[best_idx]) == std::cmp::Ordering::Greater {
                    best_idx = i;
                }
            }
            Ok(Value::Int(best_idx as i64))
        }
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
        [Value::Array(items)] => {
            let nums = collect_floats("array_argmin_float", items)?;
            let mut best_idx = 0usize;
            for (i, &v) in nums.iter().enumerate().skip(1) {
                if v.total_cmp(&nums[best_idx]) == std::cmp::Ordering::Less {
                    best_idx = i;
                }
            }
            Ok(Value::Int(best_idx as i64))
        }
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
        [Value::Array(items)] => {
            let strings = collect_strings("array_argmax_string", items)?;
            let mut best_idx = 0usize;
            for (i, s) in strings.iter().enumerate().skip(1) {
                if *s > strings[best_idx] {
                    best_idx = i;
                }
            }
            Ok(Value::Int(best_idx as i64))
        }
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
        [Value::Array(items)] => {
            let strings = collect_strings("array_argmin_string", items)?;
            let mut best_idx = 0usize;
            for (i, s) in strings.iter().enumerate().skip(1) {
                if *s < strings[best_idx] {
                    best_idx = i;
                }
            }
            Ok(Value::Int(best_idx as i64))
        }
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
