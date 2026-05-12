//! RES-1168: precision-sensitive float math primitives.
//!
//! Four pure leaf builtins that round out the math surface:
//!
//! | Builtin | Stdlib | Purpose |
//! |---|---|---|
//! | `expm1(x)`         | `f64::exp_m1`  | `exp(x) - 1`, precision for small x |
//! | `ln_1p(x)`         | `f64::ln_1p`   | `ln(1 + x)`, precision for small x |
//! | `mul_add(a, b, c)` | `f64::mul_add` | `a*b + c` with single rounding (FMA) |
//! | `recip(x)`         | `f64::recip`   | `1.0 / x` with explicit NaN/inf handling |
//!
//! Accept Int input (coerced to Float) for symmetry with the existing
//! math builtins (`sqrt`, `sin`, etc).

use crate::{RResult, Value};

fn to_f64(name: &str, v: &Value) -> RResult<f64> {
    match v {
        Value::Float(f) => Ok(*f),
        Value::Int(n) => Ok(*n as f64),
        other => Err(format!("{}: expected Float or Int, got {}", name, other)),
    }
}

/// `expm1(x) -> Float` — `exp(x) - 1`, preserving precision when `x`
/// is near zero. Mirrors `f64::exp_m1`.
pub(crate) fn builtin_expm1(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let x = to_f64("expm1", v)?;
            Ok(Value::Float(x.exp_m1()))
        }
        _ => Err(format!("expm1: expected 1 argument, got {}", args.len())),
    }
}

/// `ln_1p(x) -> Float` — `ln(1 + x)`, preserving precision when `x`
/// is near zero. Mirrors `f64::ln_1p`. `x <= -1` produces NaN (matches
/// the underlying stdlib) since `ln(0)` and below is undefined.
pub(crate) fn builtin_ln_1p(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let x = to_f64("ln_1p", v)?;
            Ok(Value::Float(x.ln_1p()))
        }
        _ => Err(format!("ln_1p: expected 1 argument, got {}", args.len())),
    }
}

/// `mul_add(a, b, c) -> Float` — `a * b + c` with a single rounding
/// step (fused multiply-add). On platforms with hardware FMA support
/// this avoids the intermediate-rounding precision loss of doing
/// the multiply and add separately. Mirrors `f64::mul_add`.
pub(crate) fn builtin_mul_add(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b, c] => {
            let av = to_f64("mul_add", a)?;
            let bv = to_f64("mul_add", b)?;
            let cv = to_f64("mul_add", c)?;
            Ok(Value::Float(av.mul_add(bv, cv)))
        }
        _ => Err(format!("mul_add: expected 3 arguments, got {}", args.len())),
    }
}

/// `recip(x) -> Float` — multiplicative inverse: `1.0 / x`. Mirrors
/// `f64::recip`. `x == 0.0` produces `±inf`; NaN propagates.
pub(crate) fn builtin_recip(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let x = to_f64("recip", v)?;
            Ok(Value::Float(x.recip()))
        }
        _ => Err(format!("recip: expected 1 argument, got {}", args.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_float(v: Value) -> f64 {
        match v {
            Value::Float(f) => f,
            other => panic!("expected Float, got {:?}", other),
        }
    }

    fn close(a: f64, b: f64) -> bool {
        if a.is_nan() && b.is_nan() {
            return true;
        }
        (a - b).abs() < 1e-12
    }

    // --- expm1 ---

    #[test]
    fn expm1_zero_is_zero() {
        assert_eq!(as_float(builtin_expm1(&[Value::Float(0.0)]).unwrap()), 0.0);
    }

    #[test]
    fn expm1_small_value_precision() {
        // exp(1e-10) - 1 ≈ 1e-10. Naive computation gives 0 due to
        // float precision; expm1 preserves the small value.
        let x = 1e-10;
        let r = as_float(builtin_expm1(&[Value::Float(x)]).unwrap());
        assert!(close(r, x), "expm1({}) = {} (expected ~{})", x, r, x);
        // Naive comparison: exp(1e-10) - 1 might round to 0.
        let naive = x.exp() - 1.0;
        // The stdlib expm1 should be at least as accurate as naive.
        assert!(
            (r - x).abs() <= (naive - x).abs().max(1e-25),
            "expm1 less accurate than naive"
        );
    }

    #[test]
    fn expm1_matches_exp_minus_one_for_large_x() {
        let x = 1.0;
        let r = as_float(builtin_expm1(&[Value::Float(x)]).unwrap());
        let expected = x.exp() - 1.0;
        assert!(close(r, expected));
    }

    #[test]
    fn expm1_int_passthrough() {
        // expm1(0) coerced from Int.
        assert_eq!(as_float(builtin_expm1(&[Value::Int(0)]).unwrap()), 0.0);
    }

    // --- ln_1p ---

    #[test]
    fn ln_1p_zero_is_zero() {
        assert_eq!(as_float(builtin_ln_1p(&[Value::Float(0.0)]).unwrap()), 0.0);
    }

    #[test]
    fn ln_1p_small_value_precision() {
        // ln(1 + 1e-10) ≈ 1e-10.
        let x = 1e-10;
        let r = as_float(builtin_ln_1p(&[Value::Float(x)]).unwrap());
        assert!(close(r, x), "ln_1p({}) = {} (expected ~{})", x, r, x);
    }

    #[test]
    fn ln_1p_matches_ln_for_large_x() {
        let x = 1.0;
        let r = as_float(builtin_ln_1p(&[Value::Float(x)]).unwrap());
        // ln(1 + 1) = ln(2)
        assert!(close(r, 2.0f64.ln()));
    }

    #[test]
    fn ln_1p_negative_one_is_neg_inf() {
        let r = as_float(builtin_ln_1p(&[Value::Float(-1.0)]).unwrap());
        assert!(r.is_infinite() && r < 0.0);
    }

    #[test]
    fn ln_1p_below_negative_one_is_nan() {
        let r = as_float(builtin_ln_1p(&[Value::Float(-2.0)]).unwrap());
        assert!(r.is_nan());
    }

    // --- mul_add ---

    #[test]
    fn mul_add_basic() {
        // 2*3 + 4 = 10.
        assert_eq!(
            as_float(
                builtin_mul_add(&[Value::Float(2.0), Value::Float(3.0), Value::Float(4.0)])
                    .unwrap()
            ),
            10.0
        );
    }

    #[test]
    fn mul_add_with_zeros() {
        assert_eq!(
            as_float(
                builtin_mul_add(&[Value::Float(0.0), Value::Float(5.0), Value::Float(7.0)])
                    .unwrap()
            ),
            7.0
        );
        assert_eq!(
            as_float(
                builtin_mul_add(&[Value::Float(5.0), Value::Float(0.0), Value::Float(7.0)])
                    .unwrap()
            ),
            7.0
        );
    }

    #[test]
    fn mul_add_negative() {
        assert_eq!(
            as_float(
                builtin_mul_add(&[Value::Float(-2.0), Value::Float(3.0), Value::Float(1.0)])
                    .unwrap()
            ),
            -5.0
        );
    }

    #[test]
    fn mul_add_int_inputs_coerce() {
        assert_eq!(
            as_float(builtin_mul_add(&[Value::Int(2), Value::Int(3), Value::Int(4)]).unwrap()),
            10.0
        );
    }

    #[test]
    fn mul_add_rejects_wrong_arity() {
        let err = builtin_mul_add(&[Value::Float(1.0), Value::Float(2.0)]).unwrap_err();
        assert!(err.contains("expected 3"));
    }

    #[test]
    fn mul_add_rejects_non_numeric() {
        let err = builtin_mul_add(&[Value::Float(1.0), Value::Bool(true), Value::Float(3.0)])
            .unwrap_err();
        assert!(err.contains("expected Float or Int"));
    }

    // --- recip ---

    #[test]
    fn recip_basic() {
        assert_eq!(as_float(builtin_recip(&[Value::Float(2.0)]).unwrap()), 0.5);
        assert_eq!(as_float(builtin_recip(&[Value::Float(4.0)]).unwrap()), 0.25);
        assert_eq!(
            as_float(builtin_recip(&[Value::Float(-2.0)]).unwrap()),
            -0.5
        );
    }

    #[test]
    fn recip_of_one_is_one() {
        assert_eq!(as_float(builtin_recip(&[Value::Float(1.0)]).unwrap()), 1.0);
    }

    #[test]
    fn recip_of_zero_is_infinity() {
        let r = as_float(builtin_recip(&[Value::Float(0.0)]).unwrap());
        assert!(r.is_infinite() && r > 0.0);
        let r = as_float(builtin_recip(&[Value::Float(-0.0)]).unwrap());
        assert!(r.is_infinite() && r < 0.0);
    }

    #[test]
    fn recip_of_nan_is_nan() {
        let r = as_float(builtin_recip(&[Value::Float(f64::NAN)]).unwrap());
        assert!(r.is_nan());
    }

    #[test]
    fn recip_double_is_identity() {
        // recip(recip(x)) ≈ x for non-zero x.
        for &x in &[2.0f64, -3.5, 0.001, 1e20] {
            let r = as_float(builtin_recip(&[Value::Float(x)]).unwrap());
            let back = as_float(builtin_recip(&[Value::Float(r)]).unwrap());
            assert!(
                close(back, x),
                "recip(recip({})) = {} (expected ~{})",
                x,
                back,
                x
            );
        }
    }

    #[test]
    fn recip_int_input() {
        assert_eq!(as_float(builtin_recip(&[Value::Int(4)]).unwrap()), 0.25);
    }

    // --- general ---

    #[test]
    fn arity_diagnostics_consistent() {
        for f in [builtin_expm1, builtin_ln_1p, builtin_recip] {
            let err = f(&[]).unwrap_err();
            assert!(err.contains("expected 1"), "got {}", err);
        }
        let err = builtin_mul_add(&[]).unwrap_err();
        assert!(err.contains("expected 3"));
    }

    #[test]
    fn type_diagnostics_consistent() {
        for f in [builtin_expm1, builtin_ln_1p, builtin_recip] {
            let err = f(&[Value::Bool(true)]).unwrap_err();
            assert!(err.contains("expected Float or Int"), "got {}", err);
        }
    }
}
