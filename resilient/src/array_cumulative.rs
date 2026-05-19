//! RES-1170: cumulative reductions + combined min/max.
//!
//! Four pure leaf builtins:
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `array_cumsum(arr)`  | `(Array) -> Array` | Prefix sums |
//! | `array_cumprod(arr)` | `(Array) -> Array` | Prefix products |
//! | `array_diffs(arr)`   | `(Array) -> Array` | Adjacent differences |
//! | `array_min_max(arr)` | `(Array) -> Array` | `[min, max]` in one pass |

use crate::{RResult, Value};

// RES-2028: extract a `Value::Int` from a `&Value` or produce a typed
// error mentioning the builtin name. Used inline by each of the four
// builtins instead of the previous `collect_ints` helper that
// materialized a fresh `Vec<i64>` per call.
#[inline]
fn as_int(name: &str, v: &Value) -> RResult<i64> {
    match v {
        Value::Int(n) => Ok(*n),
        other => Err(format!(
            "{}: expected all int elements, got {}",
            name, other
        )),
    }
}

/// `array_cumsum(arr) -> Array` — prefix sums. Output length matches
/// input length. Uses wrapping arithmetic on overflow (matches the
/// existing `array_sum` reduction convention).
pub(crate) fn builtin_array_cumsum(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            // RES-2028: single-pass scan. Inline the type-check into
            // the accumulator loop instead of materializing a
            // `Vec<i64>` first via `collect_ints`. Saves an 8*N-byte
            // throwaway allocation per call.
            let mut out: Vec<Value> = Vec::with_capacity(items.len());
            let mut acc: i64 = 0;
            for v in items {
                let n = as_int("array_cumsum", v)?;
                acc = acc.wrapping_add(n);
                out.push(Value::Int(acc));
            }
            Ok(Value::Array(out))
        }
        [other] => Err(format!("array_cumsum: expected array, got {}", other)),
        _ => Err(format!(
            "array_cumsum: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_cumprod(arr) -> Array` — prefix products. Output length
/// matches input length. Wrapping arithmetic on overflow.
pub(crate) fn builtin_array_cumprod(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            // RES-2028: single-pass scan — see comment in `array_cumsum`.
            let mut out: Vec<Value> = Vec::with_capacity(items.len());
            let mut acc: i64 = 1;
            for v in items {
                let n = as_int("array_cumprod", v)?;
                acc = acc.wrapping_mul(n);
                out.push(Value::Int(acc));
            }
            Ok(Value::Array(out))
        }
        [other] => Err(format!("array_cumprod: expected array, got {}", other)),
        _ => Err(format!(
            "array_cumprod: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_diffs(arr) -> Array` — adjacent differences:
/// `diffs(a)[i] = a[i+1] - a[i]`. Output length is `max(0, len - 1)`.
/// Wrapping subtraction on overflow.
pub(crate) fn builtin_array_diffs(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            // RES-2028: walk items pairwise with a `prev` register
            // instead of collecting into `Vec<i64>` and using
            // `.windows(2)` on it. Same algorithmic cost; drops the
            // intermediate Vec.
            if items.len() < 2 {
                return Ok(Value::Array(Vec::new()));
            }
            let mut out: Vec<Value> = Vec::with_capacity(items.len() - 1);
            let mut prev = as_int("array_diffs", &items[0])?;
            for v in items.iter().skip(1) {
                let curr = as_int("array_diffs", v)?;
                out.push(Value::Int(curr.wrapping_sub(prev)));
                prev = curr;
            }
            Ok(Value::Array(out))
        }
        [other] => Err(format!("array_diffs: expected array, got {}", other)),
        _ => Err(format!(
            "array_diffs: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_min_max(arr) -> Array[Int]` — `[min, max]` computed in a
/// single pass. Empty input is a typed error.
pub(crate) fn builtin_array_min_max(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            if items.is_empty() {
                return Err("array_min_max: empty array has no min or max".to_string());
            }
            // RES-2028: track min/max in registers during a single
            // typed scan. Previously materialized the entire input
            // as `Vec<i64>` then did the same scan.
            let first = as_int("array_min_max", &items[0])?;
            let mut min = first;
            let mut max = first;
            for v in items.iter().skip(1) {
                let n = as_int("array_min_max", v)?;
                if n < min {
                    min = n;
                }
                if n > max {
                    max = n;
                }
            }
            Ok(Value::Array(vec![Value::Int(min), Value::Int(max)]))
        }
        [other] => Err(format!("array_min_max: expected array, got {}", other)),
        _ => Err(format!(
            "array_min_max: expected 1 argument, got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ints(xs: &[i64]) -> Value {
        Value::Array(xs.iter().map(|&n| Value::Int(n)).collect())
    }

    fn as_int_vec(v: Value) -> Vec<i64> {
        match v {
            Value::Array(items) => items
                .into_iter()
                .map(|x| match x {
                    Value::Int(n) => n,
                    other => panic!("expected Int, got {:?}", other),
                })
                .collect(),
            other => panic!("expected Array, got {:?}", other),
        }
    }

    // --- cumsum ---

    #[test]
    fn cumsum_basic() {
        let r = builtin_array_cumsum(&[ints(&[1, 2, 3, 4])]).unwrap();
        assert_eq!(as_int_vec(r), vec![1, 3, 6, 10]);
    }

    #[test]
    fn cumsum_empty() {
        let r = builtin_array_cumsum(&[ints(&[])]).unwrap();
        assert_eq!(as_int_vec(r), Vec::<i64>::new());
    }

    #[test]
    fn cumsum_singleton() {
        let r = builtin_array_cumsum(&[ints(&[42])]).unwrap();
        assert_eq!(as_int_vec(r), vec![42]);
    }

    #[test]
    fn cumsum_negative_values() {
        let r = builtin_array_cumsum(&[ints(&[1, -1, 1, -1])]).unwrap();
        assert_eq!(as_int_vec(r), vec![1, 0, 1, 0]);
    }

    #[test]
    fn cumsum_zeros_preserve_value() {
        let r = builtin_array_cumsum(&[ints(&[5, 0, 0, 0])]).unwrap();
        assert_eq!(as_int_vec(r), vec![5, 5, 5, 5]);
    }

    // --- cumprod ---

    #[test]
    fn cumprod_basic() {
        let r = builtin_array_cumprod(&[ints(&[1, 2, 3, 4])]).unwrap();
        assert_eq!(as_int_vec(r), vec![1, 2, 6, 24]);
    }

    #[test]
    fn cumprod_with_zero_zeros_out() {
        let r = builtin_array_cumprod(&[ints(&[1, 2, 0, 3, 4])]).unwrap();
        assert_eq!(as_int_vec(r), vec![1, 2, 0, 0, 0]);
    }

    #[test]
    fn cumprod_empty() {
        let r = builtin_array_cumprod(&[ints(&[])]).unwrap();
        assert_eq!(as_int_vec(r), Vec::<i64>::new());
    }

    // --- diffs ---

    #[test]
    fn diffs_basic() {
        let r = builtin_array_diffs(&[ints(&[1, 3, 6, 10])]).unwrap();
        assert_eq!(as_int_vec(r), vec![2, 3, 4]);
    }

    #[test]
    fn diffs_round_trip_with_cumsum() {
        // For any sequence: cumsum + diffs = original (modulo first element).
        let original = vec![5i64, 1, -3, 7, 2];
        let sums = as_int_vec(builtin_array_cumsum(&[ints(&original)]).unwrap());
        let recovered = as_int_vec(builtin_array_diffs(&[ints(&sums)]).unwrap());
        // diffs of cumsum = original[1..]
        assert_eq!(recovered, original[1..]);
    }

    #[test]
    fn diffs_empty_input_is_empty() {
        let r = builtin_array_diffs(&[ints(&[])]).unwrap();
        assert_eq!(as_int_vec(r), Vec::<i64>::new());
    }

    #[test]
    fn diffs_singleton_is_empty() {
        let r = builtin_array_diffs(&[ints(&[42])]).unwrap();
        assert_eq!(as_int_vec(r), Vec::<i64>::new());
    }

    #[test]
    fn diffs_constant_is_zeros() {
        let r = builtin_array_diffs(&[ints(&[7, 7, 7, 7])]).unwrap();
        assert_eq!(as_int_vec(r), vec![0, 0, 0]);
    }

    // --- min_max ---

    #[test]
    fn min_max_basic() {
        let r = builtin_array_min_max(&[ints(&[3, 1, 5, 2, 4])]).unwrap();
        assert_eq!(as_int_vec(r), vec![1, 5]);
    }

    #[test]
    fn min_max_singleton() {
        let r = builtin_array_min_max(&[ints(&[42])]).unwrap();
        assert_eq!(as_int_vec(r), vec![42, 42]);
    }

    #[test]
    fn min_max_constant_array() {
        let r = builtin_array_min_max(&[ints(&[5, 5, 5])]).unwrap();
        assert_eq!(as_int_vec(r), vec![5, 5]);
    }

    #[test]
    fn min_max_extreme_values() {
        let r = builtin_array_min_max(&[ints(&[i64::MIN, 0, i64::MAX])]).unwrap();
        assert_eq!(as_int_vec(r), vec![i64::MIN, i64::MAX]);
    }

    #[test]
    fn min_max_empty_errors() {
        let err = builtin_array_min_max(&[ints(&[])]).unwrap_err();
        assert!(err.contains("empty array"));
    }

    // --- general ---

    #[test]
    fn arity_diagnostics_consistent() {
        for f in [
            builtin_array_cumsum,
            builtin_array_cumprod,
            builtin_array_diffs,
            builtin_array_min_max,
        ] {
            let err = f(&[]).unwrap_err();
            assert!(err.contains("expected 1"), "got {}", err);
            let err = f(&[Value::Int(5)]).unwrap_err();
            assert!(err.contains("expected array"), "got {}", err);
        }
    }

    #[test]
    fn rejects_mixed_element_types() {
        let arr = Value::Array(vec![Value::Int(1), Value::Float(2.0)]);
        let err = builtin_array_cumsum(std::slice::from_ref(&arr)).unwrap_err();
        assert!(err.contains("expected all int elements"));
    }

    #[test]
    fn cumsum_last_equals_array_sum() {
        // cumsum(a)[-1] == sum(a) — wrapping if overflow.
        for arr in [vec![1i64, 2, 3, 4, 5], vec![10, -5, 3, -1], vec![100; 50]] {
            let sums = as_int_vec(builtin_array_cumsum(&[ints(&arr)]).unwrap());
            let manual_sum: i64 = arr.iter().fold(0i64, |acc, &n| acc.wrapping_add(n));
            assert_eq!(*sums.last().unwrap(), manual_sum);
        }
    }
}
