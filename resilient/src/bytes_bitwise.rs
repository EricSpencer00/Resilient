//! RES-1134: bitwise + construction ops on `Value::Bytes`.
//!
//! Six pure, side-effect-free builtins that round out the byte-buffer
//! surface alongside the existing `bytes_len`, `bytes_slice`,
//! `bytes_concat`, `bytes_eq`, `bytes_starts_with`, `bytes_ends_with`,
//! `bytes_index_of`, `bytes_to_hex`, and `bytes_from_hex`. They cover
//! the gap that crypto / protocol / mask-and-merge work hits as soon
//! as the existing accessors aren't enough:
//!
//! | Builtin | Purpose |
//! |---|---|
//! | `bytes_xor(a, b) -> Bytes` | One-time-pad / stream-cipher mask / parity |
//! | `bytes_and(a, b) -> Bytes` | Selective bit clear (mask AND) |
//! | `bytes_or(a, b) -> Bytes`  | Selective bit set (mask OR) |
//! | `bytes_not(b) -> Bytes`    | Bit complement / parity helper |
//! | `bytes_fill(n, byte) -> Bytes` | Zero / sentinel buffer of length n |
//! | `bytes_reverse(b) -> Bytes` | Endianness flip, palindrome checks |
//!
//! All six are deterministic, allocate exactly one fresh `Vec<u8>` per
//! call, and never panic — every error path returns a typed `RResult`
//! diagnostic the interpreter wraps with the call-site span.

use crate::{RResult, Value};

/// `bytes_xor(a, b) -> Bytes` — element-wise XOR of two equal-length
/// `Bytes`. Errors on length mismatch — XOR of differing-length buffers
/// is almost always a bug (truncating one side hides the surprise).
pub(crate) fn builtin_bytes_xor(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(a), Value::Bytes(b)] => {
            if a.len() != b.len() {
                return Err(format!(
                    "bytes_xor: length mismatch — left={}, right={}",
                    a.len(),
                    b.len()
                ));
            }
            let out: Vec<u8> = a.iter().zip(b.iter()).map(|(x, y)| x ^ y).collect();
            Ok(Value::Bytes(out))
        }
        [Value::Bytes(_), other] => Err(format!(
            "bytes_xor: second argument must be Bytes, got {}",
            other
        )),
        [other, _] => Err(format!(
            "bytes_xor: first argument must be Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_xor: expected 2 arguments (bytes, bytes), got {}",
            args.len()
        )),
    }
}

/// `bytes_and(a, b) -> Bytes` — element-wise AND of two equal-length
/// `Bytes`. Same length contract as `bytes_xor`.
pub(crate) fn builtin_bytes_and(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(a), Value::Bytes(b)] => {
            if a.len() != b.len() {
                return Err(format!(
                    "bytes_and: length mismatch — left={}, right={}",
                    a.len(),
                    b.len()
                ));
            }
            let out: Vec<u8> = a.iter().zip(b.iter()).map(|(x, y)| x & y).collect();
            Ok(Value::Bytes(out))
        }
        [Value::Bytes(_), other] => Err(format!(
            "bytes_and: second argument must be Bytes, got {}",
            other
        )),
        [other, _] => Err(format!(
            "bytes_and: first argument must be Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_and: expected 2 arguments (bytes, bytes), got {}",
            args.len()
        )),
    }
}

/// `bytes_or(a, b) -> Bytes` — element-wise OR of two equal-length
/// `Bytes`. Same length contract as `bytes_xor`.
pub(crate) fn builtin_bytes_or(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(a), Value::Bytes(b)] => {
            if a.len() != b.len() {
                return Err(format!(
                    "bytes_or: length mismatch — left={}, right={}",
                    a.len(),
                    b.len()
                ));
            }
            let out: Vec<u8> = a.iter().zip(b.iter()).map(|(x, y)| x | y).collect();
            Ok(Value::Bytes(out))
        }
        [Value::Bytes(_), other] => Err(format!(
            "bytes_or: second argument must be Bytes, got {}",
            other
        )),
        [other, _] => Err(format!(
            "bytes_or: first argument must be Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_or: expected 2 arguments (bytes, bytes), got {}",
            args.len()
        )),
    }
}

/// `bytes_not(b) -> Bytes` — bitwise complement of every byte. Round-trip
/// holds: `bytes_not(bytes_not(b)) == b`.
pub(crate) fn builtin_bytes_not(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b)] => {
            let out: Vec<u8> = b.iter().map(|x| !x).collect();
            Ok(Value::Bytes(out))
        }
        [other] => Err(format!("bytes_not: expected Bytes, got {}", other)),
        _ => Err(format!(
            "bytes_not: expected 1 argument (bytes), got {}",
            args.len()
        )),
    }
}

/// `bytes_fill(n, byte) -> Bytes` — fresh `Bytes` of length `n` with every
/// position set to `byte`. `n` must be non-negative; `byte` must fit in a
/// `u8` (0..=255). Useful for zero buffers, sentinel fills, and padding.
pub(crate) fn builtin_bytes_fill(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n), Value::Int(byte)] => {
            if *n < 0 {
                return Err(format!(
                    "bytes_fill: length must be non-negative, got {}",
                    n
                ));
            }
            if *byte < 0 || *byte > 255 {
                return Err(format!("bytes_fill: byte must be in 0..=255, got {}", byte));
            }
            Ok(Value::Bytes(vec![*byte as u8; *n as usize]))
        }
        [a, b] => Err(format!(
            "bytes_fill: expected (Int, Int), got ({:?}, {:?})",
            a, b
        )),
        _ => Err(format!(
            "bytes_fill: expected 2 arguments (length, byte), got {}",
            args.len()
        )),
    }
}

/// `bytes_reverse(b) -> Bytes` — fresh `Bytes` with the byte order flipped.
/// Involution: `bytes_reverse(bytes_reverse(b)) == b`. Useful for
/// endianness conversion of opaque byte strings and palindrome checks.
pub(crate) fn builtin_bytes_reverse(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b)] => {
            let mut out = b.clone();
            out.reverse();
            Ok(Value::Bytes(out))
        }
        [other] => Err(format!("bytes_reverse: expected Bytes, got {}", other)),
        _ => Err(format!(
            "bytes_reverse: expected 1 argument (bytes), got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(v: &[u8]) -> Value {
        Value::Bytes(v.to_vec())
    }

    fn bytes(v: Value) -> Vec<u8> {
        match v {
            Value::Bytes(b) => b,
            other => panic!("expected Value::Bytes, got {:?}", other),
        }
    }

    #[test]
    fn xor_pair_known_vector() {
        let r = builtin_bytes_xor(&[val(&[0xFF, 0x0F, 0xAA]), val(&[0xAA, 0xAA, 0xAA])]).unwrap();
        assert_eq!(bytes(r), vec![0x55, 0xA5, 0x00]);
    }

    #[test]
    fn xor_self_is_zero() {
        let buf = val(&[1, 2, 3, 4, 5]);
        let r = builtin_bytes_xor(&[buf.clone(), buf]).unwrap();
        assert_eq!(bytes(r), vec![0, 0, 0, 0, 0]);
    }

    #[test]
    fn xor_empty_yields_empty() {
        let r = builtin_bytes_xor(&[val(&[]), val(&[])]).unwrap();
        assert_eq!(bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn xor_length_mismatch_errors() {
        let err = builtin_bytes_xor(&[val(&[1, 2]), val(&[1, 2, 3])]).unwrap_err();
        assert!(err.contains("length mismatch"), "got {}", err);
    }

    #[test]
    fn xor_round_trips_with_key() {
        let msg = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let key = vec![0x12, 0x34, 0x56, 0x78];
        let ct = builtin_bytes_xor(&[val(&msg), val(&key)]).unwrap();
        let back = builtin_bytes_xor(&[ct, val(&key)]).unwrap();
        assert_eq!(bytes(back), msg);
    }

    #[test]
    fn xor_rejects_non_bytes() {
        let err = builtin_bytes_xor(&[val(&[1]), Value::Int(7)]).unwrap_err();
        assert!(err.contains("second argument must be Bytes"));
        let err = builtin_bytes_xor(&[Value::Int(7), val(&[1])]).unwrap_err();
        assert!(err.contains("first argument must be Bytes"));
    }

    #[test]
    fn xor_arity_check() {
        let err = builtin_bytes_xor(&[val(&[1])]).unwrap_err();
        assert!(err.contains("expected 2"));
    }

    #[test]
    fn and_clears_with_zero_mask() {
        let r = builtin_bytes_and(&[val(&[0xFF, 0xFF, 0xFF]), val(&[0x00, 0x0F, 0xF0])]).unwrap();
        assert_eq!(bytes(r), vec![0x00, 0x0F, 0xF0]);
    }

    #[test]
    fn and_identity_with_all_ones() {
        let r = builtin_bytes_and(&[val(&[0xAB, 0xCD, 0xEF]), val(&[0xFF, 0xFF, 0xFF])]).unwrap();
        assert_eq!(bytes(r), vec![0xAB, 0xCD, 0xEF]);
    }

    #[test]
    fn and_length_mismatch_errors() {
        let err = builtin_bytes_and(&[val(&[1]), val(&[1, 2])]).unwrap_err();
        assert!(err.contains("length mismatch"));
    }

    #[test]
    fn or_sets_bits_with_mask() {
        let r = builtin_bytes_or(&[val(&[0x10, 0x20, 0x30]), val(&[0x0F, 0x0F, 0x0F])]).unwrap();
        assert_eq!(bytes(r), vec![0x1F, 0x2F, 0x3F]);
    }

    #[test]
    fn or_identity_with_zero_mask() {
        let r = builtin_bytes_or(&[val(&[0xAB, 0xCD, 0xEF]), val(&[0x00, 0x00, 0x00])]).unwrap();
        assert_eq!(bytes(r), vec![0xAB, 0xCD, 0xEF]);
    }

    #[test]
    fn or_length_mismatch_errors() {
        let err = builtin_bytes_or(&[val(&[1, 2, 3]), val(&[1])]).unwrap_err();
        assert!(err.contains("length mismatch"));
    }

    #[test]
    fn not_flips_each_bit() {
        let r = builtin_bytes_not(&[val(&[0x00, 0xFF, 0xAA, 0x55])]).unwrap();
        assert_eq!(bytes(r), vec![0xFF, 0x00, 0x55, 0xAA]);
    }

    #[test]
    fn not_is_involution() {
        let original = vec![0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        let once = builtin_bytes_not(&[val(&original)]).unwrap();
        let twice = builtin_bytes_not(&[once]).unwrap();
        assert_eq!(bytes(twice), original);
    }

    #[test]
    fn not_empty_yields_empty() {
        let r = builtin_bytes_not(&[val(&[])]).unwrap();
        assert_eq!(bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn not_rejects_non_bytes() {
        let err = builtin_bytes_not(&[Value::Int(7)]).unwrap_err();
        assert!(err.contains("expected Bytes"));
    }

    #[test]
    fn fill_basic() {
        let r = builtin_bytes_fill(&[Value::Int(4), Value::Int(0x42)]).unwrap();
        assert_eq!(bytes(r), vec![0x42, 0x42, 0x42, 0x42]);
    }

    #[test]
    fn fill_zero_length_is_empty() {
        let r = builtin_bytes_fill(&[Value::Int(0), Value::Int(0)]).unwrap();
        assert_eq!(bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn fill_with_byte_zero_and_ff_boundaries() {
        let r = builtin_bytes_fill(&[Value::Int(3), Value::Int(0)]).unwrap();
        assert_eq!(bytes(r), vec![0, 0, 0]);
        let r = builtin_bytes_fill(&[Value::Int(3), Value::Int(255)]).unwrap();
        assert_eq!(bytes(r), vec![0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn fill_rejects_negative_length() {
        let err = builtin_bytes_fill(&[Value::Int(-1), Value::Int(0)]).unwrap_err();
        assert!(err.contains("non-negative"));
    }

    #[test]
    fn fill_rejects_out_of_range_byte() {
        let err = builtin_bytes_fill(&[Value::Int(2), Value::Int(256)]).unwrap_err();
        assert!(err.contains("0..=255"));
        let err = builtin_bytes_fill(&[Value::Int(2), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("0..=255"));
    }

    #[test]
    fn fill_rejects_wrong_types() {
        let err = builtin_bytes_fill(&[Value::Bool(true), Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected (Int, Int)"));
    }

    #[test]
    fn reverse_known_vector() {
        let r = builtin_bytes_reverse(&[val(&[1, 2, 3, 4, 5])]).unwrap();
        assert_eq!(bytes(r), vec![5, 4, 3, 2, 1]);
    }

    #[test]
    fn reverse_is_involution() {
        let original = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x42];
        let once = builtin_bytes_reverse(&[val(&original)]).unwrap();
        let twice = builtin_bytes_reverse(&[once]).unwrap();
        assert_eq!(bytes(twice), original);
    }

    #[test]
    fn reverse_palindrome_unchanged() {
        let palindrome = vec![1, 2, 3, 2, 1];
        let r = builtin_bytes_reverse(&[val(&palindrome)]).unwrap();
        assert_eq!(bytes(r), palindrome);
    }

    #[test]
    fn reverse_empty_and_single() {
        let r = builtin_bytes_reverse(&[val(&[])]).unwrap();
        assert_eq!(bytes(r), Vec::<u8>::new());
        let r = builtin_bytes_reverse(&[val(&[0x42])]).unwrap();
        assert_eq!(bytes(r), vec![0x42]);
    }

    #[test]
    fn reverse_rejects_non_bytes() {
        let err = builtin_bytes_reverse(&[Value::Int(7)]).unwrap_err();
        assert!(err.contains("expected Bytes"));
    }

    #[test]
    fn arity_diagnostics_consistent() {
        for f in [builtin_bytes_xor, builtin_bytes_and, builtin_bytes_or] {
            let err = f(&[val(&[1])]).unwrap_err();
            assert!(err.contains("expected 2"), "got {}", err);
        }
        for f in [builtin_bytes_not, builtin_bytes_reverse] {
            let err = f(&[]).unwrap_err();
            assert!(err.contains("expected 1"), "got {}", err);
        }
        let err = builtin_bytes_fill(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected 2"));
    }
}
