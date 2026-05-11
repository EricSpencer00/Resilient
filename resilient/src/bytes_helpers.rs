//! RES-1152: `bytes_repeat`, `bytes_count_byte`, `bytes_replace_byte`.
//!
//! Three per-byte primitives that complement RES-1134's bitwise ops
//! and the existing search / accessor / construction surface.
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `bytes_repeat(b, n)`         | `(Bytes, Int) -> Bytes` | Concatenate `b` to itself `n` times |
//! | `bytes_count_byte(b, byte)`  | `(Bytes, Int) -> Int`   | Number of bytes equal to `byte` |
//! | `bytes_replace_byte(b, old, new)` | `(Bytes, Int, Int) -> Bytes` | Fresh Bytes with every `old` replaced by `new` |
//!
//! Length cap on `bytes_repeat` is 1 GiB to match `array_cycle` and
//! the other repeating builtins.

use crate::{RResult, Value};

const MAX_BYTES_REPEAT: usize = 1_000_000_000;

fn check_byte(name: &str, n: i64) -> Result<u8, String> {
    if !(0..=255).contains(&n) {
        return Err(format!("{}: byte must be in 0..=255, got {}", name, n));
    }
    Ok(n as u8)
}

/// `bytes_repeat(b, n) -> Bytes` — concatenate `b` with itself `n`
/// times. `n` must be non-negative. `n = 0` returns empty Bytes.
/// Output length capped at 1 GiB to avoid runaway memory use.
pub(crate) fn builtin_bytes_repeat(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b), Value::Int(n)] => {
            if *n < 0 {
                return Err(format!(
                    "bytes_repeat: count must be non-negative, got {}",
                    n
                ));
            }
            let count = *n as usize;
            let total = b.len().saturating_mul(count);
            if total > MAX_BYTES_REPEAT {
                return Err(format!(
                    "bytes_repeat: total length {} would exceed cap of {}",
                    total, MAX_BYTES_REPEAT
                ));
            }
            let mut out = Vec::with_capacity(total);
            for _ in 0..count {
                out.extend_from_slice(b);
            }
            Ok(Value::Bytes(out))
        }
        [Value::Bytes(_), other] => Err(format!("bytes_repeat: count must be Int, got {}", other)),
        [other, _] => Err(format!(
            "bytes_repeat: first argument must be Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_repeat: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `bytes_count_byte(b, byte) -> Int` — count of bytes in `b` equal
/// to `byte`. `byte` must be in `0..=255`.
pub(crate) fn builtin_bytes_count_byte(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b), Value::Int(byte)] => {
            let target = check_byte("bytes_count_byte", *byte)?;
            let count = b.iter().filter(|&&x| x == target).count();
            Ok(Value::Int(count as i64))
        }
        [Value::Bytes(_), other] => {
            Err(format!("bytes_count_byte: byte must be Int, got {}", other))
        }
        [other, _] => Err(format!(
            "bytes_count_byte: first argument must be Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_count_byte: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `bytes_replace_byte(b, old, new) -> Bytes` — fresh Bytes with every
/// occurrence of `old` replaced by `new`. Both `old` and `new` must be
/// in `0..=255`. Input is never mutated.
pub(crate) fn builtin_bytes_replace_byte(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b), Value::Int(old), Value::Int(new)] => {
            let from = check_byte("bytes_replace_byte", *old)?;
            let to = check_byte("bytes_replace_byte", *new)?;
            let out: Vec<u8> = b.iter().map(|&x| if x == from { to } else { x }).collect();
            Ok(Value::Bytes(out))
        }
        [Value::Bytes(_), Value::Int(_), other] => Err(format!(
            "bytes_replace_byte: new byte must be Int, got {}",
            other
        )),
        [Value::Bytes(_), other, _] => Err(format!(
            "bytes_replace_byte: old byte must be Int, got {}",
            other
        )),
        [other, _, _] => Err(format!(
            "bytes_replace_byte: first argument must be Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_replace_byte: expected 3 arguments, got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(xs: &[u8]) -> Value {
        Value::Bytes(xs.to_vec())
    }

    fn as_bytes(v: Value) -> Vec<u8> {
        match v {
            Value::Bytes(b) => b,
            other => panic!("expected Bytes, got {:?}", other),
        }
    }

    fn as_int(v: Value) -> i64 {
        match v {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    // --- bytes_repeat ---

    #[test]
    fn repeat_basic() {
        let r = builtin_bytes_repeat(&[bytes(&[1, 2]), Value::Int(3)]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 1, 2, 1, 2]);
    }

    #[test]
    fn repeat_zero_is_empty() {
        let r = builtin_bytes_repeat(&[bytes(&[1, 2, 3]), Value::Int(0)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn repeat_empty_input_stays_empty() {
        let r = builtin_bytes_repeat(&[bytes(&[]), Value::Int(5)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn repeat_one_is_identity() {
        let r = builtin_bytes_repeat(&[bytes(&[7, 8, 9]), Value::Int(1)]).unwrap();
        assert_eq!(as_bytes(r), vec![7, 8, 9]);
    }

    #[test]
    fn repeat_rejects_negative() {
        let err = builtin_bytes_repeat(&[bytes(&[1]), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("non-negative"));
    }

    #[test]
    fn repeat_caps_total_length() {
        // Length 1, count 2B → exceeds 1 GiB cap.
        let err = builtin_bytes_repeat(&[bytes(&[1]), Value::Int(2_000_000_000)]).unwrap_err();
        assert!(err.contains("exceed cap"));
    }

    #[test]
    fn repeat_rejects_wrong_arg_types() {
        let err = builtin_bytes_repeat(&[bytes(&[1]), Value::Bool(true)]).unwrap_err();
        assert!(err.contains("count must be Int"));
        let err = builtin_bytes_repeat(&[Value::Int(5), Value::Int(2)]).unwrap_err();
        assert!(err.contains("first argument must be Bytes"));
    }

    // --- bytes_count_byte ---

    #[test]
    fn count_byte_basic() {
        let b = bytes(&[1, 2, 1, 3, 1, 4]);
        assert_eq!(
            as_int(builtin_bytes_count_byte(&[b.clone(), Value::Int(1)]).unwrap()),
            3
        );
        assert_eq!(
            as_int(builtin_bytes_count_byte(&[b.clone(), Value::Int(2)]).unwrap()),
            1
        );
        assert_eq!(
            as_int(builtin_bytes_count_byte(&[b, Value::Int(99)]).unwrap()),
            0
        );
    }

    #[test]
    fn count_byte_in_empty_bytes_is_zero() {
        let r = builtin_bytes_count_byte(&[bytes(&[]), Value::Int(0)]).unwrap();
        assert_eq!(as_int(r), 0);
    }

    #[test]
    fn count_byte_zero_byte() {
        let r = builtin_bytes_count_byte(&[bytes(&[0, 1, 0, 0, 2]), Value::Int(0)]).unwrap();
        assert_eq!(as_int(r), 3);
    }

    #[test]
    fn count_byte_full_byte_range() {
        let r = builtin_bytes_count_byte(&[bytes(&[255, 255, 0]), Value::Int(255)]).unwrap();
        assert_eq!(as_int(r), 2);
    }

    #[test]
    fn count_byte_rejects_out_of_range() {
        let err = builtin_bytes_count_byte(&[bytes(&[1]), Value::Int(256)]).unwrap_err();
        assert!(err.contains("0..=255"));
        let err = builtin_bytes_count_byte(&[bytes(&[1]), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("0..=255"));
    }

    #[test]
    fn count_byte_rejects_non_int_target() {
        let err = builtin_bytes_count_byte(&[bytes(&[1]), Value::Bool(false)]).unwrap_err();
        assert!(err.contains("byte must be Int"));
    }

    // --- bytes_replace_byte ---

    #[test]
    fn replace_byte_basic() {
        let r = builtin_bytes_replace_byte(&[bytes(&[1, 2, 1, 3]), Value::Int(1), Value::Int(99)])
            .unwrap();
        assert_eq!(as_bytes(r), vec![99, 2, 99, 3]);
    }

    #[test]
    fn replace_byte_no_match_unchanged() {
        let r = builtin_bytes_replace_byte(&[bytes(&[1, 2, 3]), Value::Int(99), Value::Int(0)])
            .unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn replace_byte_identity_when_old_equals_new() {
        let r =
            builtin_bytes_replace_byte(&[bytes(&[1, 2, 3]), Value::Int(2), Value::Int(2)]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn replace_byte_empty_input_stays_empty() {
        let r = builtin_bytes_replace_byte(&[bytes(&[]), Value::Int(0), Value::Int(1)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn replace_byte_does_not_mutate_input() {
        let original = bytes(&[1, 2, 1]);
        let _ =
            builtin_bytes_replace_byte(&[original.clone(), Value::Int(1), Value::Int(9)]).unwrap();
        // Original still has 1s in place.
        match original {
            Value::Bytes(b) => assert_eq!(b, vec![1, 2, 1]),
            _ => panic!("expected Bytes"),
        }
    }

    #[test]
    fn replace_byte_rejects_out_of_range() {
        let err =
            builtin_bytes_replace_byte(&[bytes(&[1]), Value::Int(256), Value::Int(0)]).unwrap_err();
        assert!(err.contains("0..=255"));
        let err =
            builtin_bytes_replace_byte(&[bytes(&[1]), Value::Int(0), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("0..=255"));
    }

    // --- arity / type ---

    #[test]
    fn arity_diagnostics_consistent() {
        let err = builtin_bytes_repeat(&[]).unwrap_err();
        assert!(err.contains("expected 2"));
        let err = builtin_bytes_count_byte(&[bytes(&[])]).unwrap_err();
        assert!(err.contains("expected 2"));
        let err = builtin_bytes_replace_byte(&[bytes(&[]), Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected 3"));
    }

    // --- composite property ---

    #[test]
    fn repeat_then_count_matches() {
        // bytes_count_byte(bytes_repeat([0xAA], n), 0xAA) == n.
        for &n in &[0, 1, 5, 100] {
            let r = builtin_bytes_repeat(&[bytes(&[0xAA]), Value::Int(n)]).unwrap();
            assert_eq!(
                as_int(builtin_bytes_count_byte(&[r, Value::Int(0xAA)]).unwrap()),
                n
            );
        }
    }
}
