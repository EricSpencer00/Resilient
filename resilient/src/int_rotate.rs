//! RES-1182: integer bit rotation + scalar signum.
//!
//! Three pure leaf builtins that round out the integer bit-ops family
//! from RES-907 (`count_ones`, `count_zeros`, `leading_zeros`,
//! `trailing_zeros`, `swap_bytes`) and the sign family from RES-555
//! (`array_signum_int`):
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `rotate_left(x, n)`  | `(Int, Int) -> Int` | Bitwise rotate-left over the 64-bit pattern, wrapping bits from the high end to the low end |
//! | `rotate_right(x, n)` | `(Int, Int) -> Int` | Bitwise rotate-right, wrapping bits from the low end to the high end |
//! | `signum(x)`          | `(Int) -> Int` | Returns `-1`, `0`, or `+1` for negative / zero / positive `x` |
//!
//! The rotation builtins delegate to `i64::rotate_left` /
//! `i64::rotate_right` which take `n: u32` and apply `n %
//! BITS` internally — so `rotate_left(x, 64) == x` and any
//! non-negative rotation amount is well-defined. A negative `n` is a
//! typed error: there is no sensible "rotate by minus k" semantics
//! that doesn't surprise the caller.

use crate::{RResult, Value};

/// `rotate_left(x, n) -> Int` — circular left-shift of `x` by `n` bit
/// positions over its 64-bit two's-complement representation. Bits
/// shifted out of the top wrap around to the bottom; `n` is taken
/// modulo 64 internally, so `rotate_left(x, 64) == x` and
/// `rotate_left(x, 65) == rotate_left(x, 1)`.
///
/// Errors if `n` is negative — there is no `rotate_left(x, -1)`
/// convention that is not a footgun (some libraries silently make it a
/// rotate-right; that is the kind of "looks fine, off-by-one in
/// production" bug we refuse to ship).
pub(crate) fn builtin_rotate_left(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(x), Value::Int(n)] => {
            if *n < 0 {
                return Err(format!(
                    "rotate_left: rotation count must be non-negative, got {}",
                    n
                ));
            }
            // `as u32` after the non-negative check truncates n modulo
            // 2^32. Rust's `rotate_left` then applies (n_u32 % 64), so
            // any non-negative input is well-defined.
            Ok(Value::Int(x.rotate_left(*n as u32)))
        }
        [a, b] => Err(format!(
            "rotate_left: expected (int, int), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "rotate_left: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `rotate_right(x, n) -> Int` — circular right-shift of `x` by `n`
/// bit positions over its 64-bit representation. Bits shifted out of
/// the bottom wrap around to the top; `n` is taken modulo 64
/// internally.
///
/// Errors if `n` is negative for the same reason as `rotate_left`.
pub(crate) fn builtin_rotate_right(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(x), Value::Int(n)] => {
            if *n < 0 {
                return Err(format!(
                    "rotate_right: rotation count must be non-negative, got {}",
                    n
                ));
            }
            Ok(Value::Int(x.rotate_right(*n as u32)))
        }
        [a, b] => Err(format!(
            "rotate_right: expected (int, int), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "rotate_right: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `signum(x) -> Int` — returns `-1` if `x < 0`, `0` if `x == 0`,
/// `+1` if `x > 0`. Scalar counterpart of `array_signum_int`
/// (RES-555).
pub(crate) fn builtin_signum(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(x)] => Ok(Value::Int(x.signum())),
        [other] => Err(format!("signum: expected int, got {}", other)),
        _ => Err(format!("signum: expected 1 argument, got {}", args.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_int(v: Value) -> i64 {
        match v {
            Value::Int(i) => i,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    // ---------- rotate_left ----------

    #[test]
    fn rotate_left_identity_zero() {
        assert_eq!(
            ok_int(builtin_rotate_left(&[Value::Int(0x1234_5678), Value::Int(0)]).unwrap()),
            0x1234_5678
        );
    }

    #[test]
    fn rotate_left_full_cycle_is_identity() {
        assert_eq!(
            ok_int(builtin_rotate_left(&[Value::Int(0x1234_5678), Value::Int(64)]).unwrap()),
            0x1234_5678
        );
    }

    #[test]
    fn rotate_left_modulo_64() {
        // 65 == 1 mod 64
        let r1 = ok_int(builtin_rotate_left(&[Value::Int(0x1234_5678), Value::Int(1)]).unwrap());
        let r65 = ok_int(builtin_rotate_left(&[Value::Int(0x1234_5678), Value::Int(65)]).unwrap());
        assert_eq!(r1, r65);
    }

    #[test]
    fn rotate_left_low_bit_wraps_to_top() {
        // 1 rotated left by 1 = 2 (bit moves up one)
        assert_eq!(
            ok_int(builtin_rotate_left(&[Value::Int(1), Value::Int(1)]).unwrap()),
            2
        );
        // The top bit set then rotated left by 1 wraps to bit 0.
        // i64::MIN is 1 << 63 in two's complement.
        assert_eq!(
            ok_int(builtin_rotate_left(&[Value::Int(i64::MIN), Value::Int(1)]).unwrap()),
            1
        );
    }

    #[test]
    fn rotate_left_negative_count_errors() {
        let err = builtin_rotate_left(&[Value::Int(1), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("non-negative"), "got: {}", err);
    }

    #[test]
    fn rotate_left_type_errors() {
        let err = builtin_rotate_left(&[Value::Bool(true), Value::Int(1)]).unwrap_err();
        assert!(err.contains("rotate_left"), "got: {}", err);
        let err = builtin_rotate_left(&[Value::Int(1), Value::Float(1.0)]).unwrap_err();
        assert!(err.contains("rotate_left"), "got: {}", err);
    }

    #[test]
    fn rotate_left_arity_error() {
        let err = builtin_rotate_left(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("2 arguments"), "got: {}", err);
    }

    // ---------- rotate_right ----------

    #[test]
    fn rotate_right_identity_zero() {
        assert_eq!(
            ok_int(builtin_rotate_right(&[Value::Int(0x1234_5678), Value::Int(0)]).unwrap()),
            0x1234_5678
        );
    }

    #[test]
    fn rotate_right_full_cycle_is_identity() {
        assert_eq!(
            ok_int(builtin_rotate_right(&[Value::Int(0x1234_5678), Value::Int(64)]).unwrap()),
            0x1234_5678
        );
    }

    #[test]
    fn rotate_right_low_bit_wraps_to_top() {
        // 1 rotated right by 1 = i64::MIN (wraps to top bit).
        assert_eq!(
            ok_int(builtin_rotate_right(&[Value::Int(1), Value::Int(1)]).unwrap()),
            i64::MIN
        );
    }

    #[test]
    fn rotate_right_inverse_of_left() {
        // rotate_right(rotate_left(x, n), n) == x for any non-negative n.
        for n in [0i64, 1, 7, 31, 32, 63, 64, 200] {
            let x = 0x0F0F_F0F0_DEAD_BEEFi64;
            let rotated = ok_int(builtin_rotate_left(&[Value::Int(x), Value::Int(n)]).unwrap());
            let back = ok_int(builtin_rotate_right(&[Value::Int(rotated), Value::Int(n)]).unwrap());
            assert_eq!(back, x, "round-trip failed for n={}", n);
        }
    }

    #[test]
    fn rotate_right_negative_count_errors() {
        let err = builtin_rotate_right(&[Value::Int(1), Value::Int(-7)]).unwrap_err();
        assert!(err.contains("non-negative"), "got: {}", err);
    }

    #[test]
    fn rotate_right_modulo_64() {
        let r1 = ok_int(builtin_rotate_right(&[Value::Int(0x1234_5678), Value::Int(1)]).unwrap());
        let r129 =
            ok_int(builtin_rotate_right(&[Value::Int(0x1234_5678), Value::Int(129)]).unwrap());
        assert_eq!(r1, r129); // 129 == 1 mod 64
    }

    // ---------- signum ----------

    #[test]
    fn signum_positive() {
        assert_eq!(ok_int(builtin_signum(&[Value::Int(42)]).unwrap()), 1);
        assert_eq!(ok_int(builtin_signum(&[Value::Int(i64::MAX)]).unwrap()), 1);
    }

    #[test]
    fn signum_zero() {
        assert_eq!(ok_int(builtin_signum(&[Value::Int(0)]).unwrap()), 0);
    }

    #[test]
    fn signum_negative() {
        assert_eq!(ok_int(builtin_signum(&[Value::Int(-1)]).unwrap()), -1);
        assert_eq!(ok_int(builtin_signum(&[Value::Int(i64::MIN)]).unwrap()), -1);
    }

    #[test]
    fn signum_type_error() {
        let err = builtin_signum(&[Value::Bool(false)]).unwrap_err();
        assert!(err.contains("signum"), "got: {}", err);
    }

    #[test]
    fn signum_arity_error() {
        let err = builtin_signum(&[]).unwrap_err();
        assert!(err.contains("1 argument"), "got: {}", err);
    }
}
