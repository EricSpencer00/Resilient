//! RES-1156: per-bit accessors on i64.
//!
//! Four pure leaf builtins that complement the existing whole-word bit
//! ops (`reverse_bits`, `swap_bytes`, `rotate_left_int`/`right`,
//! `count_ones`/`zeros`, `leading_zeros`/`trailing_zeros`).
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `set_bit(n, idx)`   | `(Int, Int) -> Int`  | `n` with bit `idx` set to 1 |
//! | `clear_bit(n, idx)` | `(Int, Int) -> Int`  | `n` with bit `idx` set to 0 |
//! | `get_bit(n, idx)`   | `(Int, Int) -> Bool` | True iff bit `idx` of `n` is 1 |
//! | `flip_bit(n, idx)`  | `(Int, Int) -> Int`  | `n` with bit `idx` XOR-toggled |
//!
//! `idx` is 0-based (LSB = 0). Out of `0..=63` is a typed error —
//! masks the easy "shift count too large" bug.

use crate::{RResult, Value};

fn check_bit_idx(name: &str, idx: i64) -> Result<u32, String> {
    if !(0..64).contains(&idx) {
        return Err(format!(
            "{}: bit index must be in 0..=63, got {}",
            name, idx
        ));
    }
    Ok(idx as u32)
}

/// `set_bit(n, idx) -> Int` — return `n` with bit `idx` set to 1.
/// Idempotent (calling twice returns the same value).
pub(crate) fn builtin_set_bit(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n), Value::Int(idx)] => {
            let bit = check_bit_idx("set_bit", *idx)?;
            Ok(Value::Int(*n | (1i64 << bit)))
        }
        [Value::Int(_), other] => Err(format!("set_bit: idx must be Int, got {}", other)),
        [other, _] => Err(format!("set_bit: n must be Int, got {}", other)),
        _ => Err(format!("set_bit: expected 2 arguments, got {}", args.len())),
    }
}

/// `clear_bit(n, idx) -> Int` — return `n` with bit `idx` set to 0.
/// Idempotent. Negative numbers have meaningful upper bits (i64 is
/// two's complement); clearing bit 63 toggles sign.
pub(crate) fn builtin_clear_bit(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n), Value::Int(idx)] => {
            let bit = check_bit_idx("clear_bit", *idx)?;
            Ok(Value::Int(*n & !(1i64 << bit)))
        }
        [Value::Int(_), other] => Err(format!("clear_bit: idx must be Int, got {}", other)),
        [other, _] => Err(format!("clear_bit: n must be Int, got {}", other)),
        _ => Err(format!(
            "clear_bit: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `get_bit(n, idx) -> Bool` — true iff bit `idx` of `n` is 1.
pub(crate) fn builtin_get_bit(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n), Value::Int(idx)] => {
            let bit = check_bit_idx("get_bit", *idx)?;
            Ok(Value::Bool((*n >> bit) & 1 == 1))
        }
        [Value::Int(_), other] => Err(format!("get_bit: idx must be Int, got {}", other)),
        [other, _] => Err(format!("get_bit: n must be Int, got {}", other)),
        _ => Err(format!("get_bit: expected 2 arguments, got {}", args.len())),
    }
}

/// `flip_bit(n, idx) -> Int` — return `n` with bit `idx` XOR-toggled.
/// Involution: `flip_bit(flip_bit(n, i), i) == n`.
pub(crate) fn builtin_flip_bit(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n), Value::Int(idx)] => {
            let bit = check_bit_idx("flip_bit", *idx)?;
            Ok(Value::Int(*n ^ (1i64 << bit)))
        }
        [Value::Int(_), other] => Err(format!("flip_bit: idx must be Int, got {}", other)),
        [other, _] => Err(format!("flip_bit: n must be Int, got {}", other)),
        _ => Err(format!(
            "flip_bit: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_int(v: Value) -> i64 {
        match v {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    fn as_bool(v: Value) -> bool {
        match v {
            Value::Bool(b) => b,
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    // --- set_bit ---

    #[test]
    fn set_bit_basic() {
        assert_eq!(
            as_int(builtin_set_bit(&[Value::Int(0), Value::Int(0)]).unwrap()),
            1
        );
        assert_eq!(
            as_int(builtin_set_bit(&[Value::Int(0), Value::Int(3)]).unwrap()),
            8
        );
        assert_eq!(
            as_int(builtin_set_bit(&[Value::Int(0b0101), Value::Int(1)]).unwrap()),
            0b0111
        );
    }

    #[test]
    fn set_bit_idempotent() {
        let v = builtin_set_bit(&[Value::Int(0xFF), Value::Int(3)]).unwrap();
        let v2 = builtin_set_bit(&[v.clone(), Value::Int(3)]).unwrap();
        assert_eq!(as_int(v), as_int(v2));
    }

    #[test]
    fn set_bit_at_msb() {
        let r = as_int(builtin_set_bit(&[Value::Int(0), Value::Int(63)]).unwrap());
        assert_eq!(r, i64::MIN); // bit 63 in two's complement is the sign bit
    }

    #[test]
    fn set_bit_rejects_out_of_range_idx() {
        let err = builtin_set_bit(&[Value::Int(0), Value::Int(64)]).unwrap_err();
        assert!(err.contains("0..=63"));
        let err = builtin_set_bit(&[Value::Int(0), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("0..=63"));
    }

    // --- clear_bit ---

    #[test]
    fn clear_bit_basic() {
        assert_eq!(
            as_int(builtin_clear_bit(&[Value::Int(0xFF), Value::Int(0)]).unwrap()),
            0xFE
        );
        assert_eq!(
            as_int(builtin_clear_bit(&[Value::Int(0b1111), Value::Int(2)]).unwrap()),
            0b1011
        );
    }

    #[test]
    fn clear_bit_idempotent() {
        let v = builtin_clear_bit(&[Value::Int(0xFF), Value::Int(0)]).unwrap();
        let v2 = builtin_clear_bit(&[v.clone(), Value::Int(0)]).unwrap();
        assert_eq!(as_int(v), as_int(v2));
    }

    #[test]
    fn clear_bit_at_msb_makes_positive() {
        // i64::MIN has only bit 63 set. Clearing it gives 0.
        let r = as_int(builtin_clear_bit(&[Value::Int(i64::MIN), Value::Int(63)]).unwrap());
        assert_eq!(r, 0);
    }

    #[test]
    fn clear_bit_already_zero_is_noop() {
        let r = as_int(builtin_clear_bit(&[Value::Int(0), Value::Int(5)]).unwrap());
        assert_eq!(r, 0);
    }

    // --- get_bit ---

    #[test]
    fn get_bit_basic() {
        assert!(as_bool(
            builtin_get_bit(&[Value::Int(1), Value::Int(0)]).unwrap()
        ));
        assert!(!as_bool(
            builtin_get_bit(&[Value::Int(1), Value::Int(1)]).unwrap()
        ));
        assert!(as_bool(
            builtin_get_bit(&[Value::Int(0b1010), Value::Int(1)]).unwrap()
        ));
        assert!(as_bool(
            builtin_get_bit(&[Value::Int(0b1010), Value::Int(3)]).unwrap()
        ));
        assert!(!as_bool(
            builtin_get_bit(&[Value::Int(0b1010), Value::Int(0)]).unwrap()
        ));
    }

    #[test]
    fn get_bit_msb_of_min() {
        // i64::MIN = -9223372036854775808 has bit 63 set, all others 0.
        assert!(as_bool(
            builtin_get_bit(&[Value::Int(i64::MIN), Value::Int(63)]).unwrap()
        ));
        assert!(!as_bool(
            builtin_get_bit(&[Value::Int(i64::MIN), Value::Int(0)]).unwrap()
        ));
    }

    #[test]
    fn get_bit_negative_one_all_set() {
        for idx in 0..64 {
            assert!(as_bool(
                builtin_get_bit(&[Value::Int(-1), Value::Int(idx)]).unwrap()
            ));
        }
    }

    // --- flip_bit ---

    #[test]
    fn flip_bit_basic() {
        assert_eq!(
            as_int(builtin_flip_bit(&[Value::Int(0), Value::Int(2)]).unwrap()),
            4
        );
        assert_eq!(
            as_int(builtin_flip_bit(&[Value::Int(0b101), Value::Int(0)]).unwrap()),
            0b100
        );
        assert_eq!(
            as_int(builtin_flip_bit(&[Value::Int(0b101), Value::Int(1)]).unwrap()),
            0b111
        );
    }

    #[test]
    fn flip_bit_is_involution() {
        for n in [0i64, 1, -1, 0x5A5A5A5A, i64::MAX, i64::MIN] {
            for idx in [0i64, 7, 31, 63] {
                let once = as_int(builtin_flip_bit(&[Value::Int(n), Value::Int(idx)]).unwrap());
                let twice = as_int(builtin_flip_bit(&[Value::Int(once), Value::Int(idx)]).unwrap());
                assert_eq!(
                    twice, n,
                    "flip_bit({}, {}) twice should be identity",
                    n, idx
                );
            }
        }
    }

    // --- general ---

    #[test]
    fn round_trip_set_clear_get() {
        // For any input n and idx in 0..64:
        //   get_bit(set_bit(n, i), i) == true
        //   get_bit(clear_bit(n, i), i) == false
        for n in [0i64, 0x1234_5678, -1, i64::MAX] {
            for idx in [0i64, 1, 7, 31, 63] {
                let set = as_int(builtin_set_bit(&[Value::Int(n), Value::Int(idx)]).unwrap());
                assert!(as_bool(
                    builtin_get_bit(&[Value::Int(set), Value::Int(idx)]).unwrap()
                ));
                let clr = as_int(builtin_clear_bit(&[Value::Int(n), Value::Int(idx)]).unwrap());
                assert!(!as_bool(
                    builtin_get_bit(&[Value::Int(clr), Value::Int(idx)]).unwrap()
                ));
            }
        }
    }

    #[test]
    fn arity_diagnostics_consistent() {
        for f in [
            builtin_set_bit,
            builtin_clear_bit,
            builtin_get_bit,
            builtin_flip_bit,
        ] {
            let err = f(&[]).unwrap_err();
            assert!(err.contains("expected 2"), "got {}", err);
            let err = f(&[Value::Int(0)]).unwrap_err();
            assert!(err.contains("expected 2"), "got {}", err);
            let err = f(&[Value::Bool(false), Value::Int(0)]).unwrap_err();
            assert!(err.contains("n must be Int"), "got {}", err);
            let err = f(&[Value::Int(0), Value::Bool(false)]).unwrap_err();
            assert!(err.contains("idx must be Int"), "got {}", err);
        }
    }

    #[test]
    fn out_of_range_idx_diagnostics_consistent() {
        for f in [
            builtin_set_bit,
            builtin_clear_bit,
            builtin_get_bit,
            builtin_flip_bit,
        ] {
            let err = f(&[Value::Int(0), Value::Int(64)]).unwrap_err();
            assert!(err.contains("0..=63"), "got {}", err);
        }
    }
}
