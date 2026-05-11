//! RES-1178: `bytes_take` / `bytes_drop` / `bytes_take_last` / `bytes_drop_last`.
//!
//! Four pure leaf builtins that fill the slicing surface for `Value::Bytes`,
//! parallel to RES-421 (`array_take` / `array_drop`) and RES-537
//! (`array_take_last` / `array_drop_last`) for arrays.
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `bytes_take(b, n)`      | `(Bytes, Int) -> Bytes` | First `n` bytes |
//! | `bytes_drop(b, n)`      | `(Bytes, Int) -> Bytes` | `b` minus the first `n` |
//! | `bytes_take_last(b, n)` | `(Bytes, Int) -> Bytes` | Last `n` bytes |
//! | `bytes_drop_last(b, n)` | `(Bytes, Int) -> Bytes` | `b` minus the last `n` |
//!
//! Semantics:
//! - All four require `n >= 0`. Negative `n` is a typed error.
//! - `n > len(b)` clamps to `len(b)` — no error, matches the array
//!   counterparts.
//! - `n == 0` returns the appropriate identity.
//! - All four produce a fresh `Bytes` — the input is never mutated.

use crate::{RResult, Value};

/// `bytes_take(b, n) -> Bytes` — first `n` bytes of `b`.
/// `n` must be non-negative; `n > len(b)` clamps to `len(b)`.
pub(crate) fn builtin_bytes_take(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b), Value::Int(n)] => {
            if *n < 0 {
                return Err(format!("bytes_take: count must be non-negative, got {}", n));
            }
            let take = (*n as usize).min(b.len());
            Ok(Value::Bytes(b[..take].to_vec()))
        }
        [Value::Bytes(_), other] => Err(format!("bytes_take: count must be Int, got {}", other)),
        [other, _] => Err(format!(
            "bytes_take: first argument must be Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_take: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `bytes_drop(b, n) -> Bytes` — `b` with the first `n` bytes removed.
/// `n` must be non-negative; `n >= len(b)` returns empty Bytes.
pub(crate) fn builtin_bytes_drop(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b), Value::Int(n)] => {
            if *n < 0 {
                return Err(format!("bytes_drop: count must be non-negative, got {}", n));
            }
            let drop = (*n as usize).min(b.len());
            Ok(Value::Bytes(b[drop..].to_vec()))
        }
        [Value::Bytes(_), other] => Err(format!("bytes_drop: count must be Int, got {}", other)),
        [other, _] => Err(format!(
            "bytes_drop: first argument must be Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_drop: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `bytes_take_last(b, n) -> Bytes` — last `n` bytes of `b`.
/// `n` must be non-negative; `n > len(b)` clamps to `len(b)`.
pub(crate) fn builtin_bytes_take_last(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b), Value::Int(n)] => {
            if *n < 0 {
                return Err(format!(
                    "bytes_take_last: count must be non-negative, got {}",
                    n
                ));
            }
            let take = (*n as usize).min(b.len());
            let start = b.len() - take;
            Ok(Value::Bytes(b[start..].to_vec()))
        }
        [Value::Bytes(_), other] => {
            Err(format!("bytes_take_last: count must be Int, got {}", other))
        }
        [other, _] => Err(format!(
            "bytes_take_last: first argument must be Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_take_last: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `bytes_drop_last(b, n) -> Bytes` — `b` with the last `n` bytes removed.
/// `n` must be non-negative; `n >= len(b)` returns empty Bytes.
/// Round-trip: `bytes_concat(bytes_drop_last(b, n), bytes_take_last(b, n)) == b`.
pub(crate) fn builtin_bytes_drop_last(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b), Value::Int(n)] => {
            if *n < 0 {
                return Err(format!(
                    "bytes_drop_last: count must be non-negative, got {}",
                    n
                ));
            }
            let drop = (*n as usize).min(b.len());
            let end = b.len() - drop;
            Ok(Value::Bytes(b[..end].to_vec()))
        }
        [Value::Bytes(_), other] => {
            Err(format!("bytes_drop_last: count must be Int, got {}", other))
        }
        [other, _] => Err(format!(
            "bytes_drop_last: first argument must be Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_drop_last: expected 2 arguments, got {}",
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

    // --- bytes_take ---

    #[test]
    fn take_basic() {
        let r = builtin_bytes_take(&[bytes(&[1, 2, 3, 4, 5]), Value::Int(3)]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn take_zero_returns_empty() {
        let r = builtin_bytes_take(&[bytes(&[1, 2, 3]), Value::Int(0)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn take_more_than_len_clamps() {
        let r = builtin_bytes_take(&[bytes(&[1, 2, 3]), Value::Int(99)]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn take_exact_len_returns_full() {
        let r = builtin_bytes_take(&[bytes(&[1, 2, 3]), Value::Int(3)]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn take_from_empty_is_empty() {
        let r = builtin_bytes_take(&[bytes(&[]), Value::Int(5)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn take_rejects_negative() {
        let err = builtin_bytes_take(&[bytes(&[1, 2]), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("non-negative"));
    }

    #[test]
    fn take_rejects_wrong_types() {
        let err = builtin_bytes_take(&[bytes(&[1]), Value::Bool(true)]).unwrap_err();
        assert!(err.contains("count must be Int"));
        let err = builtin_bytes_take(&[Value::Int(5), Value::Int(2)]).unwrap_err();
        assert!(err.contains("first argument must be Bytes"));
    }

    // --- bytes_drop ---

    #[test]
    fn drop_basic() {
        let r = builtin_bytes_drop(&[bytes(&[1, 2, 3, 4, 5]), Value::Int(2)]).unwrap();
        assert_eq!(as_bytes(r), vec![3, 4, 5]);
    }

    #[test]
    fn drop_zero_returns_full() {
        let r = builtin_bytes_drop(&[bytes(&[1, 2, 3]), Value::Int(0)]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn drop_more_than_len_returns_empty() {
        let r = builtin_bytes_drop(&[bytes(&[1, 2, 3]), Value::Int(99)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn drop_exact_len_returns_empty() {
        let r = builtin_bytes_drop(&[bytes(&[1, 2, 3]), Value::Int(3)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn drop_from_empty_is_empty() {
        let r = builtin_bytes_drop(&[bytes(&[]), Value::Int(5)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn drop_rejects_negative() {
        let err = builtin_bytes_drop(&[bytes(&[1, 2]), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("non-negative"));
    }

    // --- bytes_take_last ---

    #[test]
    fn take_last_basic() {
        let r = builtin_bytes_take_last(&[bytes(&[1, 2, 3, 4, 5]), Value::Int(2)]).unwrap();
        assert_eq!(as_bytes(r), vec![4, 5]);
    }

    #[test]
    fn take_last_zero_returns_empty() {
        let r = builtin_bytes_take_last(&[bytes(&[1, 2, 3]), Value::Int(0)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn take_last_more_than_len_returns_full() {
        let r = builtin_bytes_take_last(&[bytes(&[1, 2, 3]), Value::Int(99)]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn take_last_exact_len_returns_full() {
        let r = builtin_bytes_take_last(&[bytes(&[1, 2, 3]), Value::Int(3)]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn take_last_from_empty_is_empty() {
        let r = builtin_bytes_take_last(&[bytes(&[]), Value::Int(5)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn take_last_rejects_negative() {
        let err = builtin_bytes_take_last(&[bytes(&[1]), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("non-negative"));
    }

    // --- bytes_drop_last ---

    #[test]
    fn drop_last_basic() {
        let r = builtin_bytes_drop_last(&[bytes(&[1, 2, 3, 4, 5]), Value::Int(2)]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn drop_last_zero_returns_full() {
        let r = builtin_bytes_drop_last(&[bytes(&[1, 2, 3]), Value::Int(0)]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn drop_last_more_than_len_returns_empty() {
        let r = builtin_bytes_drop_last(&[bytes(&[1, 2, 3]), Value::Int(99)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn drop_last_exact_len_returns_empty() {
        let r = builtin_bytes_drop_last(&[bytes(&[1, 2, 3]), Value::Int(3)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn drop_last_from_empty_is_empty() {
        let r = builtin_bytes_drop_last(&[bytes(&[]), Value::Int(5)]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn drop_last_rejects_negative() {
        let err = builtin_bytes_drop_last(&[bytes(&[1]), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("non-negative"));
    }

    // --- arity / type diagnostics ---

    #[test]
    fn arity_diagnostics_consistent() {
        for &(label, f) in &[
            ("take", builtin_bytes_take as fn(&[Value]) -> RResult<Value>),
            ("drop", builtin_bytes_drop),
            ("take_last", builtin_bytes_take_last),
            ("drop_last", builtin_bytes_drop_last),
        ] {
            let err = f(&[]).unwrap_err();
            assert!(err.contains("expected 2"), "{}: {}", label, err);
        }
    }

    // --- composite properties ---

    #[test]
    fn take_drop_round_trip() {
        // bytes_take(b, n) ++ bytes_drop(b, n) == b, for any 0 <= n <= len(b).
        let b = vec![10u8, 20, 30, 40, 50];
        for n in 0..=b.len() {
            let head = as_bytes(builtin_bytes_take(&[bytes(&b), Value::Int(n as i64)]).unwrap());
            let tail = as_bytes(builtin_bytes_drop(&[bytes(&b), Value::Int(n as i64)]).unwrap());
            let mut joined = head;
            joined.extend(tail);
            assert_eq!(joined, b, "round-trip failed at n={}", n);
        }
    }

    #[test]
    fn take_last_drop_last_round_trip() {
        // bytes_drop_last(b, n) ++ bytes_take_last(b, n) == b.
        let b = vec![1u8, 2, 3, 4, 5, 6];
        for n in 0..=b.len() {
            let head =
                as_bytes(builtin_bytes_drop_last(&[bytes(&b), Value::Int(n as i64)]).unwrap());
            let tail =
                as_bytes(builtin_bytes_take_last(&[bytes(&b), Value::Int(n as i64)]).unwrap());
            let mut joined = head;
            joined.extend(tail);
            assert_eq!(joined, b, "round-trip failed at n={}", n);
        }
    }

    #[test]
    fn take_then_drop_equals_input() {
        // bytes_drop(b, len(b)) == empty; bytes_take(b, len(b)) == b.
        let b = vec![7u8, 8, 9];
        let len = b.len() as i64;
        assert_eq!(
            as_bytes(builtin_bytes_take(&[bytes(&b), Value::Int(len)]).unwrap()),
            b
        );
        assert_eq!(
            as_bytes(builtin_bytes_drop(&[bytes(&b), Value::Int(len)]).unwrap()),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn does_not_mutate_input() {
        let original = bytes(&[1, 2, 3, 4, 5]);
        let _ = builtin_bytes_take(&[original.clone(), Value::Int(2)]).unwrap();
        let _ = builtin_bytes_drop(&[original.clone(), Value::Int(2)]).unwrap();
        let _ = builtin_bytes_take_last(&[original.clone(), Value::Int(2)]).unwrap();
        let _ = builtin_bytes_drop_last(&[original.clone(), Value::Int(2)]).unwrap();
        match original {
            Value::Bytes(b) => assert_eq!(b, vec![1, 2, 3, 4, 5]),
            _ => panic!("expected Bytes"),
        }
    }
}
