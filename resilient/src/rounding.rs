//! RES-1166: rounding builtins — round / trunc + int-returning variants.
//!
//! `floor` and `ceil` already exist; this completes the four basic
//! rounding modes plus typed-int conversions that surface NaN / ±inf /
//! overflow as typed errors instead of silently wrapping.
//!
//! | Builtin | Signature | Mode |
//! |---|---|---|
//! | `round(x)`        | `(Float) -> Float` | Ties-to-even (IEEE 754 default) |
//! | `trunc(x)`        | `(Float) -> Float` | Toward zero |
//! | `round_to_int(x)` | `(Float) -> Int`   | Round + safe int conversion |
//! | `trunc_to_int(x)` | `(Float) -> Int`   | Trunc + safe int conversion |

use crate::{RResult, Value};

fn safe_to_int(name: &str, f: f64) -> RResult<i64> {
    if f.is_nan() {
        return Err(format!("{}: cannot convert NaN to int", name));
    }
    if f.is_infinite() {
        return Err(format!(
            "{}: cannot convert {} infinity to int",
            name,
            if f > 0.0 { "positive" } else { "negative" }
        ));
    }
    // i64 range check — f64 can represent any i64 magnitude but only
    // with precision loss past 2^53. We accept the precision loss; the
    // check here is purely a range bound.
    if f < i64::MIN as f64 || f > i64::MAX as f64 {
        return Err(format!("{}: value {} is out of i64 range", name, f));
    }
    Ok(f as i64)
}

/// `round(x) -> Float` — round `x` to the nearest integer, with
/// ties-to-even (IEEE 754 default). Returns `Float` so NaN / ±inf pass
/// through. Examples: `round(0.5) == 0.0`, `round(1.5) == 2.0`,
/// `round(2.5) == 2.0`.
pub(crate) fn builtin_round(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(x)] => Ok(Value::Float(x.round_ties_even())),
        [Value::Int(n)] => Ok(Value::Float(*n as f64)),
        [other] => Err(format!("round: expected float, got {}", other)),
        _ => Err(format!("round: expected 1 argument, got {}", args.len())),
    }
}

/// `trunc(x) -> Float` — round `x` toward zero (drop fractional part).
pub(crate) fn builtin_trunc(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(x)] => Ok(Value::Float(x.trunc())),
        [Value::Int(n)] => Ok(Value::Float(*n as f64)),
        [other] => Err(format!("trunc: expected float, got {}", other)),
        _ => Err(format!("trunc: expected 1 argument, got {}", args.len())),
    }
}

/// `round_to_int(x) -> Int` — round to nearest (ties-to-even) and
/// convert to `Int`. Errors on NaN, ±infinity, or out-of-i64-range.
pub(crate) fn builtin_round_to_int(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(x)] => {
            let rounded = x.round_ties_even();
            Ok(Value::Int(safe_to_int("round_to_int", rounded)?))
        }
        [Value::Int(n)] => Ok(Value::Int(*n)),
        [other] => Err(format!("round_to_int: expected float, got {}", other)),
        _ => Err(format!(
            "round_to_int: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `trunc_to_int(x) -> Int` — truncate toward zero and convert to `Int`.
/// Same error contract as `round_to_int`.
pub(crate) fn builtin_trunc_to_int(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(x)] => {
            let truncated = x.trunc();
            Ok(Value::Int(safe_to_int("trunc_to_int", truncated)?))
        }
        [Value::Int(n)] => Ok(Value::Int(*n)),
        [other] => Err(format!("trunc_to_int: expected float, got {}", other)),
        _ => Err(format!(
            "trunc_to_int: expected 1 argument, got {}",
            args.len()
        )),
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

    fn as_int(v: Value) -> i64 {
        match v {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    // --- round ---

    #[test]
    fn round_basic() {
        assert_eq!(as_float(builtin_round(&[Value::Float(1.4)]).unwrap()), 1.0);
        assert_eq!(as_float(builtin_round(&[Value::Float(1.6)]).unwrap()), 2.0);
        assert_eq!(
            as_float(builtin_round(&[Value::Float(-1.4)]).unwrap()),
            -1.0
        );
        assert_eq!(
            as_float(builtin_round(&[Value::Float(-1.6)]).unwrap()),
            -2.0
        );
    }

    #[test]
    fn round_ties_to_even() {
        // IEEE 754 default rounding: ties round to the even neighbor.
        assert_eq!(as_float(builtin_round(&[Value::Float(0.5)]).unwrap()), 0.0);
        assert_eq!(as_float(builtin_round(&[Value::Float(1.5)]).unwrap()), 2.0);
        assert_eq!(as_float(builtin_round(&[Value::Float(2.5)]).unwrap()), 2.0);
        assert_eq!(as_float(builtin_round(&[Value::Float(3.5)]).unwrap()), 4.0);
        assert_eq!(as_float(builtin_round(&[Value::Float(-0.5)]).unwrap()), 0.0);
        assert_eq!(
            as_float(builtin_round(&[Value::Float(-1.5)]).unwrap()),
            -2.0
        );
    }

    #[test]
    fn round_propagates_nan_and_inf() {
        let nan = as_float(builtin_round(&[Value::Float(f64::NAN)]).unwrap());
        assert!(nan.is_nan());
        assert_eq!(
            as_float(builtin_round(&[Value::Float(f64::INFINITY)]).unwrap()),
            f64::INFINITY
        );
    }

    #[test]
    fn round_accepts_int() {
        // Int arg is coerced to Float for symmetry with the existing
        // floor/ceil/sqrt convention.
        assert_eq!(as_float(builtin_round(&[Value::Int(42)]).unwrap()), 42.0);
    }

    // --- trunc ---

    #[test]
    fn trunc_basic() {
        assert_eq!(as_float(builtin_trunc(&[Value::Float(3.7)]).unwrap()), 3.0);
        assert_eq!(
            as_float(builtin_trunc(&[Value::Float(-3.7)]).unwrap()),
            -3.0
        );
        assert_eq!(
            as_float(builtin_trunc(&[Value::Float(0.999)]).unwrap()),
            0.0
        );
        assert_eq!(
            as_float(builtin_trunc(&[Value::Float(-0.999)]).unwrap()),
            0.0
        );
    }

    #[test]
    fn trunc_vs_floor_at_negatives() {
        // trunc(-1.5) = -1, but floor(-1.5) = -2.
        // Just verify trunc rounds toward zero.
        assert_eq!(
            as_float(builtin_trunc(&[Value::Float(-1.5)]).unwrap()),
            -1.0
        );
        assert_eq!(
            as_float(builtin_trunc(&[Value::Float(-2.9)]).unwrap()),
            -2.0
        );
    }

    #[test]
    fn trunc_propagates_nan_and_inf() {
        let nan = as_float(builtin_trunc(&[Value::Float(f64::NAN)]).unwrap());
        assert!(nan.is_nan());
        assert_eq!(
            as_float(builtin_trunc(&[Value::Float(f64::NEG_INFINITY)]).unwrap()),
            f64::NEG_INFINITY
        );
    }

    // --- round_to_int ---

    #[test]
    fn round_to_int_basic() {
        assert_eq!(
            as_int(builtin_round_to_int(&[Value::Float(1.4)]).unwrap()),
            1
        );
        assert_eq!(
            as_int(builtin_round_to_int(&[Value::Float(1.6)]).unwrap()),
            2
        );
        assert_eq!(
            as_int(builtin_round_to_int(&[Value::Float(0.5)]).unwrap()),
            0
        );
        assert_eq!(
            as_int(builtin_round_to_int(&[Value::Float(1.5)]).unwrap()),
            2
        );
        assert_eq!(
            as_int(builtin_round_to_int(&[Value::Float(-1.5)]).unwrap()),
            -2
        );
    }

    #[test]
    fn round_to_int_rejects_nan() {
        let err = builtin_round_to_int(&[Value::Float(f64::NAN)]).unwrap_err();
        assert!(err.contains("NaN"));
    }

    #[test]
    fn round_to_int_rejects_infinity() {
        let err = builtin_round_to_int(&[Value::Float(f64::INFINITY)]).unwrap_err();
        assert!(err.contains("infinity"));
        let err = builtin_round_to_int(&[Value::Float(f64::NEG_INFINITY)]).unwrap_err();
        assert!(err.contains("infinity"));
    }

    #[test]
    fn round_to_int_rejects_out_of_range() {
        let err = builtin_round_to_int(&[Value::Float(1e30)]).unwrap_err();
        assert!(err.contains("out of i64 range"));
        let err = builtin_round_to_int(&[Value::Float(-1e30)]).unwrap_err();
        assert!(err.contains("out of i64 range"));
    }

    #[test]
    fn round_to_int_int_passthrough() {
        assert_eq!(as_int(builtin_round_to_int(&[Value::Int(42)]).unwrap()), 42);
    }

    // --- trunc_to_int ---

    #[test]
    fn trunc_to_int_basic() {
        assert_eq!(
            as_int(builtin_trunc_to_int(&[Value::Float(3.7)]).unwrap()),
            3
        );
        assert_eq!(
            as_int(builtin_trunc_to_int(&[Value::Float(-3.7)]).unwrap()),
            -3
        );
        assert_eq!(
            as_int(builtin_trunc_to_int(&[Value::Float(0.0)]).unwrap()),
            0
        );
    }

    #[test]
    fn trunc_to_int_rejects_nan_and_inf() {
        let err = builtin_trunc_to_int(&[Value::Float(f64::NAN)]).unwrap_err();
        assert!(err.contains("NaN"));
        let err = builtin_trunc_to_int(&[Value::Float(f64::INFINITY)]).unwrap_err();
        assert!(err.contains("infinity"));
    }

    #[test]
    fn trunc_to_int_int_passthrough() {
        assert_eq!(
            as_int(builtin_trunc_to_int(&[Value::Int(-42)]).unwrap()),
            -42
        );
    }

    // --- consistency ---

    #[test]
    fn round_matches_round_to_int_modulo_type() {
        for &x in &[1.4f64, 1.6, -1.5, 0.5, 2.5, 100.0, -100.7] {
            let as_float = as_float(builtin_round(&[Value::Float(x)]).unwrap());
            let as_int = as_int(builtin_round_to_int(&[Value::Float(x)]).unwrap()) as f64;
            assert_eq!(as_float, as_int, "round({}) inconsistent across types", x);
        }
    }

    #[test]
    fn trunc_matches_trunc_to_int_modulo_type() {
        for &x in &[1.4f64, 1.9, -1.5, -2.9, 0.0] {
            let as_float = as_float(builtin_trunc(&[Value::Float(x)]).unwrap());
            let as_int = as_int(builtin_trunc_to_int(&[Value::Float(x)]).unwrap()) as f64;
            assert_eq!(as_float, as_int, "trunc({}) inconsistent across types", x);
        }
    }

    // --- general ---

    #[test]
    fn arity_diagnostics_consistent() {
        for f in [
            builtin_round,
            builtin_trunc,
            builtin_round_to_int,
            builtin_trunc_to_int,
        ] {
            let err = f(&[]).unwrap_err();
            assert!(err.contains("expected 1"), "got {}", err);
        }
    }
}
