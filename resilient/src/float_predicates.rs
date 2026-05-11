//! RES-1138: IEEE 754 classification + total order + sign-bit predicates.
//!
//! Five pure leaf builtins that complete the float-predicate surface
//! alongside the existing `is_nan`, `is_inf`, `is_finite` (RES-130) and
//! the `float_to_bits` / `float_from_bits` bit-reinterpret pair
//! (RES-1130). Each delegates to the corresponding `f64::*` stdlib
//! method, so behaviour matches Rust exactly.
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `float_classify(x)`     | `(Float) -> String` | `"normal"` / `"subnormal"` / `"zero"` / `"infinite"` / `"nan"` |
//! | `float_total_cmp(a, b)` | `(Float, Float) -> Int` | IEEE 754 total order: `-1` / `0` / `1` |
//! | `float_is_normal(x)`    | `(Float) -> Bool` | Normal (finite, non-zero, non-subnormal) |
//! | `float_is_subnormal(x)` | `(Float) -> Bool` | Subnormal (denormal) |
//! | `float_sign_bit(x)`     | `(Float) -> Bool` | Sign bit set — distinguishes `-0` from `+0` |
//!
//! `float_total_cmp` is the only IEEE 754 total order available in the
//! language; `<` returns false for any comparison involving NaN, which
//! makes sorting `Float[]` arrays containing NaNs undefined behaviour.

use crate::{RResult, Value};
use std::num::FpCategory;

/// `float_classify(x) -> String` — IEEE 754 number classification.
/// Returns one of `"normal"`, `"subnormal"`, `"zero"`, `"infinite"`,
/// `"nan"`. Mirrors `f64::classify()`.
pub(crate) fn builtin_float_classify(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(x)] => {
            let kind = match x.classify() {
                FpCategory::Nan => "nan",
                FpCategory::Infinite => "infinite",
                FpCategory::Zero => "zero",
                FpCategory::Subnormal => "subnormal",
                FpCategory::Normal => "normal",
            };
            Ok(Value::String(kind.to_string()))
        }
        [other] => Err(format!("float_classify: expected float, got {}", other)),
        _ => Err(format!(
            "float_classify: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `float_total_cmp(a, b) -> Int` — IEEE 754 total order: returns `-1`
/// if `a < b`, `0` if `a == b`, `1` if `a > b`. Unlike `<`, this orders
/// NaN values, distinguishes `-0.0` from `+0.0`, and orders negative
/// NaN payloads before positive ones. Delegates to `f64::total_cmp`.
pub(crate) fn builtin_float_total_cmp(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(a), Value::Float(b)] => {
            let ord = a.total_cmp(b) as i64;
            Ok(Value::Int(ord))
        }
        [a, b] => Err(format!(
            "float_total_cmp: expected (float, float), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "float_total_cmp: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `float_is_normal(x) -> Bool` — true iff `x` is a *normal* IEEE 754
/// double: finite, non-zero, and non-subnormal. Mirrors `f64::is_normal`.
pub(crate) fn builtin_float_is_normal(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(x)] => Ok(Value::Bool(x.is_normal())),
        [other] => Err(format!("float_is_normal: expected float, got {}", other)),
        _ => Err(format!(
            "float_is_normal: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `float_is_subnormal(x) -> Bool` — true iff `x` is subnormal
/// (a.k.a. denormal). Mirrors `f64::is_subnormal`.
pub(crate) fn builtin_float_is_subnormal(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(x)] => Ok(Value::Bool(x.is_subnormal())),
        [other] => Err(format!("float_is_subnormal: expected float, got {}", other)),
        _ => Err(format!(
            "float_is_subnormal: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `float_sign_bit(x) -> Bool` — true iff the IEEE 754 sign bit is set.
/// Distinguishes `-0.0` (returns `true`) from `+0.0` (returns `false`),
/// which the `<` operator cannot. Mirrors `f64::is_sign_negative`.
pub(crate) fn builtin_float_sign_bit(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(x)] => Ok(Value::Bool(x.is_sign_negative())),
        [other] => Err(format!("float_sign_bit: expected float, got {}", other)),
        _ => Err(format!(
            "float_sign_bit: expected 1 argument, got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(x: f64) -> String {
        match builtin_float_classify(&[Value::Float(x)]).unwrap() {
            Value::String(s) => s,
            other => panic!("expected String, got {:?}", other),
        }
    }

    fn total_cmp(a: f64, b: f64) -> i64 {
        match builtin_float_total_cmp(&[Value::Float(a), Value::Float(b)]).unwrap() {
            Value::Int(v) => v,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    fn is_normal(x: f64) -> bool {
        match builtin_float_is_normal(&[Value::Float(x)]).unwrap() {
            Value::Bool(v) => v,
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    fn is_subnormal(x: f64) -> bool {
        match builtin_float_is_subnormal(&[Value::Float(x)]).unwrap() {
            Value::Bool(v) => v,
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    fn sign_bit(x: f64) -> bool {
        match builtin_float_sign_bit(&[Value::Float(x)]).unwrap() {
            Value::Bool(v) => v,
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn classify_each_category() {
        assert_eq!(classify(1.0), "normal");
        assert_eq!(classify(-1.0), "normal");
        assert_eq!(classify(0.0), "zero");
        assert_eq!(classify(-0.0), "zero");
        assert_eq!(classify(f64::INFINITY), "infinite");
        assert_eq!(classify(f64::NEG_INFINITY), "infinite");
        assert_eq!(classify(f64::NAN), "nan");
        assert_eq!(classify(f64::MIN_POSITIVE / 2.0), "subnormal");
    }

    #[test]
    fn classify_rejects_int_and_other_types() {
        let err = builtin_float_classify(&[Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected float"));
        let err = builtin_float_classify(&[Value::Bool(true)]).unwrap_err();
        assert!(err.contains("expected float"));
    }

    #[test]
    fn classify_arity_check() {
        let err = builtin_float_classify(&[]).unwrap_err();
        assert!(err.contains("expected 1"));
        let err = builtin_float_classify(&[Value::Float(0.0), Value::Float(0.0)]).unwrap_err();
        assert!(err.contains("expected 1"));
    }

    #[test]
    fn total_cmp_basic_ordering() {
        assert_eq!(total_cmp(1.0, 2.0), -1);
        assert_eq!(total_cmp(2.0, 1.0), 1);
        assert_eq!(total_cmp(1.0, 1.0), 0);
    }

    #[test]
    fn total_cmp_distinguishes_negative_zero() {
        // IEEE 754 total order: -0.0 < +0.0
        assert_eq!(total_cmp(-0.0, 0.0), -1);
        assert_eq!(total_cmp(0.0, -0.0), 1);
        // self comparisons are equal
        assert_eq!(total_cmp(0.0, 0.0), 0);
        assert_eq!(total_cmp(-0.0, -0.0), 0);
    }

    #[test]
    fn total_cmp_orders_nan() {
        // Negative NaN < every finite < positive NaN. We test that
        // total order is strict (transitivity is property of f64::total_cmp).
        let nan = f64::NAN;
        // self-comparison: NaN totalCmp NaN must be 0 (unlike `<`)
        assert_eq!(total_cmp(nan, nan), 0);
        // ordering with infinities — positive NaN sorts after +inf
        assert_eq!(total_cmp(f64::INFINITY, nan), -1);
        // negative-NaN bit-pattern sorts before -inf
        let neg_nan = f64::from_bits(f64::NAN.to_bits() | (1 << 63));
        assert_eq!(total_cmp(neg_nan, f64::NEG_INFINITY), -1);
    }

    #[test]
    fn total_cmp_orders_infinities() {
        assert_eq!(total_cmp(f64::NEG_INFINITY, f64::INFINITY), -1);
        assert_eq!(total_cmp(f64::INFINITY, f64::NEG_INFINITY), 1);
        assert_eq!(total_cmp(f64::INFINITY, f64::INFINITY), 0);
    }

    #[test]
    fn total_cmp_rejects_wrong_arity_and_types() {
        let err = builtin_float_total_cmp(&[Value::Float(0.0)]).unwrap_err();
        assert!(err.contains("expected 2"));
        let err = builtin_float_total_cmp(&[Value::Float(0.0), Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected (float, float)"));
    }

    #[test]
    fn is_normal_basic() {
        assert!(is_normal(1.0));
        assert!(is_normal(-1.0));
        assert!(is_normal(1e300));
        // zero is NOT normal
        assert!(!is_normal(0.0));
        assert!(!is_normal(-0.0));
        // subnormals are not normal
        assert!(!is_normal(f64::MIN_POSITIVE / 2.0));
        // inf and nan are not normal
        assert!(!is_normal(f64::INFINITY));
        assert!(!is_normal(f64::NEG_INFINITY));
        assert!(!is_normal(f64::NAN));
    }

    #[test]
    fn is_subnormal_basic() {
        assert!(is_subnormal(f64::MIN_POSITIVE / 2.0));
        // 1.0 is normal, not subnormal
        assert!(!is_subnormal(1.0));
        // zero is not subnormal
        assert!(!is_subnormal(0.0));
        assert!(!is_subnormal(-0.0));
        // inf / nan are not subnormal
        assert!(!is_subnormal(f64::INFINITY));
        assert!(!is_subnormal(f64::NAN));
    }

    #[test]
    fn classification_is_partition() {
        // Every float falls into exactly one of: normal, subnormal,
        // zero, infinite, nan.
        for &x in &[
            1.0,
            -1.0,
            0.0,
            -0.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            f64::MIN_POSITIVE / 2.0,
            1e-300,
            1e300,
            f64::MIN_POSITIVE,
            f64::MAX,
        ] {
            let cat = classify(x);
            let count = [
                cat == "normal",
                cat == "subnormal",
                cat == "zero",
                cat == "infinite",
                cat == "nan",
            ]
            .iter()
            .filter(|b| **b)
            .count();
            assert_eq!(count, 1, "{} got category {}", x, cat);
        }
    }

    #[test]
    fn sign_bit_distinguishes_zeros() {
        assert!(!sign_bit(0.0));
        assert!(sign_bit(-0.0));
    }

    #[test]
    fn sign_bit_finite_values() {
        assert!(!sign_bit(1.0));
        assert!(sign_bit(-1.0));
        assert!(!sign_bit(f64::INFINITY));
        assert!(sign_bit(f64::NEG_INFINITY));
        assert!(!sign_bit(f64::MAX));
        assert!(sign_bit(f64::MIN));
    }

    #[test]
    fn sign_bit_on_negative_nan() {
        // Positive NaN has sign bit clear; negative NaN (different
        // bit pattern) has it set.
        assert!(!sign_bit(f64::NAN));
        let neg_nan = f64::from_bits(f64::NAN.to_bits() | (1 << 63));
        assert!(sign_bit(neg_nan));
    }

    #[test]
    fn classify_total_cmp_compose_for_sort() {
        // Building block: sorting a Vec<f64> with total_cmp as the
        // comparator should always produce a well-defined order even
        // with NaNs and -0.0 in the input.
        let mut values = [
            f64::NAN,
            1.0,
            -0.0,
            f64::INFINITY,
            -1.0,
            0.0,
            f64::NEG_INFINITY,
        ];
        values.sort_by(|a, b| {
            let ord = total_cmp(*a, *b);
            if ord < 0 {
                std::cmp::Ordering::Less
            } else if ord > 0 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
        // Expected total order (with positive NaN): -inf, -1, -0, 0, 1, +inf, nan
        assert_eq!(classify(values[0]), "infinite");
        assert!(sign_bit(values[0]));
        assert_eq!(values[1], -1.0);
        assert!(sign_bit(values[2]));
        assert_eq!(classify(values[2]), "zero");
        assert!(!sign_bit(values[3]));
        assert_eq!(classify(values[3]), "zero");
        assert_eq!(values[4], 1.0);
        assert_eq!(classify(values[5]), "infinite");
        assert!(!sign_bit(values[5]));
        assert_eq!(classify(values[6]), "nan");
    }

    #[test]
    fn float_predicate_arity_diagnostics_consistent() {
        for f in [
            builtin_float_is_normal,
            builtin_float_is_subnormal,
            builtin_float_sign_bit,
        ] {
            let err = f(&[]).unwrap_err();
            assert!(err.contains("expected 1"), "got {}", err);
            let err = f(&[Value::Int(0)]).unwrap_err();
            assert!(err.contains("expected float"), "got {}", err);
        }
    }
}
