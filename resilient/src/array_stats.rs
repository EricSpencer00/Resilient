//! RES-1150: statistical reductions — variance, standard deviation,
//! float-median, float-range.
//!
//! Rounds out the array-statistics surface alongside `array_sum`,
//! `array_average*`, `array_median_int`, `array_range_int`. Every
//! telemetry / monitoring / sensor-fusion workload computes the
//! second-moment statistics; today every call site implements
//! Welford-style variance by hand.
//!
//! | Builtin | Signature | Notes |
//! |---|---|---|
//! | `array_variance_int(arr)`   | `(Array) -> Float` | Population variance over int input |
//! | `array_variance_float(arr)` | `(Array) -> Float` | Population variance over float input |
//! | `array_stddev_int(arr)`     | `(Array) -> Float` | √variance |
//! | `array_stddev_float(arr)`   | `(Array) -> Float` | √variance |
//! | `array_median_float(arr)`   | `(Array) -> Float` | Median using IEEE 754 total order |
//! | `array_range_float(arr)`    | `(Array) -> Float` | max − min using total order |
//!
//! - **Population variance** (divide by N) — matches `array_average`'s
//!   "treat input as the whole dataset" convention. Sample variance
//!   (divide by N−1) is derivable by the caller.
//! - **Empty arrays are typed errors** — matches `array_min` /
//!   `array_max` / `array_median_int`. Variance / stddev / median /
//!   range are undefined on the empty set.
//! - **Float ranking** uses `f64::total_cmp` (RES-1138), so NaN / ±0
//!   are well-ordered.

use crate::{RResult, Value};

// RES-2030: inline type-check helpers. Replace the previous
// `collect_ints` / `collect_floats` helpers that materialized an
// intermediate Vec from the entire input — these are throwaway for
// the variance / stddev / range computations.

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

#[inline]
fn as_float(name: &str, v: &Value) -> RResult<f64> {
    match v {
        Value::Float(f) => Ok(*f),
        other => Err(format!(
            "{}: expected all float elements, got {}",
            name, other
        )),
    }
}

/// RES-2030: population variance computed directly over `&[Value]`
/// without an intermediate Vec. Pass 1 validates element type and
/// sums to find the mean; pass 2 re-pattern-matches the (validated)
/// slice and accumulates the sum-of-squared-deviations. Same two-pass
/// algorithm as the previous helper-based shape, no allocations.
fn population_variance_int(name: &str, items: &[Value]) -> RResult<f64> {
    if items.is_empty() {
        return Err(format!("{}: empty array has no value", name));
    }
    let mut sum: f64 = 0.0;
    for v in items {
        let n = as_int(name, v)?;
        sum += n as f64;
    }
    let n_f = items.len() as f64;
    let mean = sum / n_f;
    let mut sumsq: f64 = 0.0;
    for v in items {
        // Validated by pass 1 — silently skip otherwise (unreachable).
        if let Value::Int(n) = v {
            let x = *n as f64;
            sumsq += (x - mean).powi(2);
        }
    }
    Ok(sumsq / n_f)
}

/// Float variant of `population_variance_int`.
fn population_variance_float(name: &str, items: &[Value]) -> RResult<f64> {
    if items.is_empty() {
        return Err(format!("{}: empty array has no value", name));
    }
    let mut sum: f64 = 0.0;
    for v in items {
        let f = as_float(name, v)?;
        sum += f;
    }
    let n_f = items.len() as f64;
    let mean = sum / n_f;
    let mut sumsq: f64 = 0.0;
    for v in items {
        if let Value::Float(f) = v {
            sumsq += (*f - mean).powi(2);
        }
    }
    Ok(sumsq / n_f)
}

/// `array_variance_int(arr) -> Float` — population variance
/// (∑(xᵢ - μ)² / N) of an int array. Computation is performed in `f64`.
pub(crate) fn builtin_array_variance_int(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => Ok(Value::Float(population_variance_int(
            "array_variance_int",
            items,
        )?)),
        [other] => Err(format!("array_variance_int: expected array, got {}", other)),
        _ => Err(format!(
            "array_variance_int: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_variance_float(arr) -> Float` — population variance of a
/// float array.
pub(crate) fn builtin_array_variance_float(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => Ok(Value::Float(population_variance_float(
            "array_variance_float",
            items,
        )?)),
        [other] => Err(format!(
            "array_variance_float: expected array, got {}",
            other
        )),
        _ => Err(format!(
            "array_variance_float: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_stddev_int(arr) -> Float` — √(population variance) of an
/// int array.
pub(crate) fn builtin_array_stddev_int(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => Ok(Value::Float(
            population_variance_int("array_stddev_int", items)?.sqrt(),
        )),
        [other] => Err(format!("array_stddev_int: expected array, got {}", other)),
        _ => Err(format!(
            "array_stddev_int: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_stddev_float(arr) -> Float` — √(population variance) of a
/// float array.
pub(crate) fn builtin_array_stddev_float(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => Ok(Value::Float(
            population_variance_float("array_stddev_float", items)?.sqrt(),
        )),
        [other] => Err(format!("array_stddev_float: expected array, got {}", other)),
        _ => Err(format!(
            "array_stddev_float: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_median_float(arr) -> Float` — median value using IEEE 754
/// total order. Odd-length: middle element after sorting. Even-length:
/// average of the two middle elements. NaN-safe.
pub(crate) fn builtin_array_median_float(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            // RES-2030: median needs sorted random access, so we still
            // materialize the `Vec<f64>` — but the type-check inlines
            // (no separate `collect_floats` helper).
            if items.is_empty() {
                return Err("array_median_float: empty array has no value".to_string());
            }
            let mut nums: Vec<f64> = Vec::with_capacity(items.len());
            for v in items {
                nums.push(as_float("array_median_float", v)?);
            }
            nums.sort_by(|a, b| a.total_cmp(b));
            let n = nums.len();
            let median = if n % 2 == 1 {
                nums[n / 2]
            } else {
                (nums[n / 2 - 1] + nums[n / 2]) / 2.0
            };
            Ok(Value::Float(median))
        }
        [other] => Err(format!("array_median_float: expected array, got {}", other)),
        _ => Err(format!(
            "array_median_float: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_range_float(arr) -> Float` — peak-to-peak (max − min) using
/// IEEE 754 total order for ranking. NaN-safe.
pub(crate) fn builtin_array_range_float(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            // RES-2030: single-pass min/max scan with inline type-check.
            // Previously materialized a `Vec<f64>` via `collect_floats`
            // before scanning — pure overhead since we only need two
            // floats (min, max) tracked in registers.
            if items.is_empty() {
                return Err("array_range_float: empty array has no value".to_string());
            }
            let first = as_float("array_range_float", &items[0])?;
            let mut min = first;
            let mut max = first;
            for v in items.iter().skip(1) {
                let f = as_float("array_range_float", v)?;
                if f.total_cmp(&min) == std::cmp::Ordering::Less {
                    min = f;
                }
                if f.total_cmp(&max) == std::cmp::Ordering::Greater {
                    max = f;
                }
            }
            Ok(Value::Float(max - min))
        }
        [other] => Err(format!("array_range_float: expected array, got {}", other)),
        _ => Err(format!(
            "array_range_float: expected 1 argument, got {}",
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

    fn floats(xs: &[f64]) -> Value {
        Value::Array(xs.iter().map(|&f| Value::Float(f)).collect())
    }

    fn as_float(v: Value) -> f64 {
        match v {
            Value::Float(f) => f,
            other => panic!("expected Float, got {:?}", other),
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // --- variance ---

    #[test]
    fn variance_int_known_values() {
        // [1, 2, 3, 4, 5] — mean 3, variance 2.0
        let r = builtin_array_variance_int(&[ints(&[1, 2, 3, 4, 5])]).unwrap();
        assert!(close(as_float(r), 2.0));
    }

    #[test]
    fn variance_int_constant_array_is_zero() {
        let r = builtin_array_variance_int(&[ints(&[7, 7, 7, 7])]).unwrap();
        assert_eq!(as_float(r), 0.0);
    }

    #[test]
    fn variance_int_single_element_is_zero() {
        let r = builtin_array_variance_int(&[ints(&[42])]).unwrap();
        assert_eq!(as_float(r), 0.0);
    }

    #[test]
    fn variance_float_known_values() {
        // [1.0, 1.0, 5.0, 5.0] — mean 3, variance 4.0
        let r = builtin_array_variance_float(&[floats(&[1.0, 1.0, 5.0, 5.0])]).unwrap();
        assert!(close(as_float(r), 4.0));
    }

    #[test]
    fn variance_rejects_empty() {
        let err = builtin_array_variance_int(&[ints(&[])]).unwrap_err();
        assert!(err.contains("empty array"));
        let err = builtin_array_variance_float(&[floats(&[])]).unwrap_err();
        assert!(err.contains("empty array"));
    }

    // --- stddev ---

    #[test]
    fn stddev_int_matches_sqrt_variance() {
        let arr = ints(&[2, 4, 4, 4, 5, 5, 7, 9]);
        let var = as_float(builtin_array_variance_int(std::slice::from_ref(&arr)).unwrap());
        let sd = as_float(builtin_array_stddev_int(&[arr]).unwrap());
        assert!(close(sd, var.sqrt()));
        // Known: this is the classic "8 numbers" example, stddev = 2.0.
        assert!(close(sd, 2.0));
    }

    #[test]
    fn stddev_float_matches_sqrt_variance() {
        let arr = floats(&[1.5, 2.5, 3.5, 4.5]);
        let var = as_float(builtin_array_variance_float(std::slice::from_ref(&arr)).unwrap());
        let sd = as_float(builtin_array_stddev_float(&[arr]).unwrap());
        assert!(close(sd, var.sqrt()));
    }

    #[test]
    fn stddev_constant_is_zero() {
        let r = builtin_array_stddev_int(&[ints(&[10, 10, 10])]).unwrap();
        assert_eq!(as_float(r), 0.0);
    }

    #[test]
    fn stddev_rejects_empty() {
        let err = builtin_array_stddev_int(&[ints(&[])]).unwrap_err();
        assert!(err.contains("empty array"));
        let err = builtin_array_stddev_float(&[floats(&[])]).unwrap_err();
        assert!(err.contains("empty array"));
    }

    // --- median_float ---

    #[test]
    fn median_float_odd_length() {
        let r = builtin_array_median_float(&[floats(&[3.0, 1.0, 5.0])]).unwrap();
        assert_eq!(as_float(r), 3.0);
    }

    #[test]
    fn median_float_even_length() {
        let r = builtin_array_median_float(&[floats(&[1.0, 2.0, 3.0, 4.0])]).unwrap();
        assert_eq!(as_float(r), 2.5);
    }

    #[test]
    fn median_float_unsorted_input() {
        // 7.0 is the median of [9, 2, 7, 5, 1] when sorted: [1, 2, 5, 7, 9]
        let r = builtin_array_median_float(&[floats(&[9.0, 2.0, 7.0, 5.0, 1.0])]).unwrap();
        assert_eq!(as_float(r), 5.0);
    }

    #[test]
    fn median_float_single_element() {
        let r = builtin_array_median_float(&[floats(&[42.0])]).unwrap();
        assert_eq!(as_float(r), 42.0);
    }

    #[test]
    fn median_float_rejects_empty() {
        let err = builtin_array_median_float(&[floats(&[])]).unwrap_err();
        assert!(err.contains("empty array"));
    }

    // --- range_float ---

    #[test]
    fn range_float_basic() {
        let r = builtin_array_range_float(&[floats(&[1.0, 5.0, 2.0, 8.0, 3.0])]).unwrap();
        assert_eq!(as_float(r), 7.0); // 8 - 1
    }

    #[test]
    fn range_float_negative_to_positive() {
        let r = builtin_array_range_float(&[floats(&[-10.0, -5.0, 0.0, 5.0, 10.0])]).unwrap();
        assert_eq!(as_float(r), 20.0);
    }

    #[test]
    fn range_float_constant_is_zero() {
        let r = builtin_array_range_float(&[floats(&[7.0, 7.0, 7.0])]).unwrap();
        assert_eq!(as_float(r), 0.0);
    }

    #[test]
    fn range_float_single_element_is_zero() {
        let r = builtin_array_range_float(&[floats(&[5.0])]).unwrap();
        assert_eq!(as_float(r), 0.0);
    }

    #[test]
    fn range_float_rejects_empty() {
        let err = builtin_array_range_float(&[floats(&[])]).unwrap_err();
        assert!(err.contains("empty array"));
    }

    // --- type / arity ---

    #[test]
    fn arity_diagnostics_consistent() {
        for f in [
            builtin_array_variance_int,
            builtin_array_variance_float,
            builtin_array_stddev_int,
            builtin_array_stddev_float,
            builtin_array_median_float,
            builtin_array_range_float,
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
            builtin_array_variance_int(&[Value::Array(vec![Value::Int(1), Value::Float(2.0)])])
                .unwrap_err();
        assert!(err.contains("expected all int elements"));

        let err =
            builtin_array_variance_float(&[Value::Array(vec![Value::Float(1.0), Value::Int(2)])])
                .unwrap_err();
        assert!(err.contains("expected all float elements"));
    }
}
