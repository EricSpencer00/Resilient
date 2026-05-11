//! RES-1176: bytes_strip_prefix / bytes_strip_suffix / bytes_to_string.
//!
//! Three pure leaf builtins that close gaps in the Bytes ↔ String
//! conversion surface:
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `bytes_strip_prefix(b, prefix)` | `(Bytes, Bytes) -> Bytes`         | Strip prefix if matches; else return b unchanged |
//! | `bytes_strip_suffix(b, suffix)` | `(Bytes, Bytes) -> Bytes`         | Strip suffix if matches; else return b unchanged |
//! | `bytes_to_string(b)`            | `(Bytes) -> Result<String, String>` | UTF-8 decode |

use crate::{RResult, Value};

/// `bytes_strip_prefix(b, prefix) -> Bytes` — return `b` with `prefix`
/// removed from the front if it matches. Returns `b` unchanged
/// otherwise (matches `string_strip_prefix` ergonomics — no Option
/// wrap).
pub(crate) fn builtin_bytes_strip_prefix(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b), Value::Bytes(prefix)] => {
            let stripped = match b.strip_prefix(prefix.as_slice()) {
                Some(rest) => rest.to_vec(),
                None => b.clone(),
            };
            Ok(Value::Bytes(stripped))
        }
        [Value::Bytes(_), other] => Err(format!(
            "bytes_strip_prefix: second argument must be Bytes, got {}",
            other
        )),
        [other, _] => Err(format!(
            "bytes_strip_prefix: first argument must be Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_strip_prefix: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `bytes_strip_suffix(b, suffix) -> Bytes` — return `b` with `suffix`
/// removed from the end if it matches. Returns `b` unchanged otherwise.
pub(crate) fn builtin_bytes_strip_suffix(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b), Value::Bytes(suffix)] => {
            let stripped = match b.strip_suffix(suffix.as_slice()) {
                Some(rest) => rest.to_vec(),
                None => b.clone(),
            };
            Ok(Value::Bytes(stripped))
        }
        [Value::Bytes(_), other] => Err(format!(
            "bytes_strip_suffix: second argument must be Bytes, got {}",
            other
        )),
        [other, _] => Err(format!(
            "bytes_strip_suffix: first argument must be Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_strip_suffix: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `bytes_to_string(b) -> Result<String, String>` — UTF-8 decode.
/// `Ok(s)` if the bytes are valid UTF-8, `Err(msg)` otherwise.
/// Mirrors `string_from_bytes` (RES-566) which takes `Array<Int>` —
/// this is the direct `Bytes` overload.
pub(crate) fn builtin_bytes_to_string(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b)] => match std::str::from_utf8(b) {
            Ok(s) => Ok(Value::Result {
                ok: true,
                payload: Box::new(Value::String(s.to_string())),
            }),
            Err(e) => Ok(Value::Result {
                ok: false,
                payload: Box::new(Value::String(format!(
                    "invalid UTF-8 at byte {}",
                    e.valid_up_to()
                ))),
            }),
        },
        [other] => Err(format!("bytes_to_string: expected Bytes, got {}", other)),
        _ => Err(format!(
            "bytes_to_string: expected 1 argument, got {}",
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

    fn as_result(v: Value) -> (bool, Value) {
        match v {
            Value::Result { ok, payload } => (ok, *payload),
            other => panic!("expected Result, got {:?}", other),
        }
    }

    fn as_string(v: Value) -> String {
        match v {
            Value::String(s) => s,
            other => panic!("expected String, got {:?}", other),
        }
    }

    // --- strip_prefix ---

    #[test]
    fn strip_prefix_match_returns_suffix() {
        let r = builtin_bytes_strip_prefix(&[bytes(&[1, 2, 3, 4, 5]), bytes(&[1, 2])]).unwrap();
        assert_eq!(as_bytes(r), vec![3, 4, 5]);
    }

    #[test]
    fn strip_prefix_no_match_returns_unchanged() {
        let r = builtin_bytes_strip_prefix(&[bytes(&[1, 2, 3]), bytes(&[9, 9])]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn strip_prefix_empty_prefix_is_identity() {
        let r = builtin_bytes_strip_prefix(&[bytes(&[1, 2, 3]), bytes(&[])]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn strip_prefix_full_match_returns_empty() {
        let r = builtin_bytes_strip_prefix(&[bytes(&[1, 2, 3]), bytes(&[1, 2, 3])]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    #[test]
    fn strip_prefix_longer_than_input_no_match() {
        let r = builtin_bytes_strip_prefix(&[bytes(&[1]), bytes(&[1, 2, 3])]).unwrap();
        assert_eq!(as_bytes(r), vec![1]);
    }

    #[test]
    fn strip_prefix_rejects_wrong_type() {
        let err = builtin_bytes_strip_prefix(&[bytes(&[1]), Value::Int(2)]).unwrap_err();
        assert!(err.contains("second argument"));
        let err = builtin_bytes_strip_prefix(&[Value::Int(1), bytes(&[1])]).unwrap_err();
        assert!(err.contains("first argument"));
    }

    // --- strip_suffix ---

    #[test]
    fn strip_suffix_match_returns_prefix() {
        let r = builtin_bytes_strip_suffix(&[bytes(&[1, 2, 3, 4, 5]), bytes(&[4, 5])]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn strip_suffix_no_match_returns_unchanged() {
        let r = builtin_bytes_strip_suffix(&[bytes(&[1, 2, 3]), bytes(&[9, 9])]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn strip_suffix_empty_suffix_is_identity() {
        let r = builtin_bytes_strip_suffix(&[bytes(&[1, 2, 3]), bytes(&[])]).unwrap();
        assert_eq!(as_bytes(r), vec![1, 2, 3]);
    }

    #[test]
    fn strip_suffix_full_match_returns_empty() {
        let r = builtin_bytes_strip_suffix(&[bytes(&[1, 2, 3]), bytes(&[1, 2, 3])]).unwrap();
        assert_eq!(as_bytes(r), Vec::<u8>::new());
    }

    // --- bytes_to_string ---

    #[test]
    fn to_string_ascii_round_trip() {
        let r = builtin_bytes_to_string(&[bytes(b"hello")]).unwrap();
        let (ok, payload) = as_result(r);
        assert!(ok);
        assert_eq!(as_string(payload), "hello");
    }

    #[test]
    fn to_string_empty_is_ok_empty() {
        let r = builtin_bytes_to_string(&[bytes(&[])]).unwrap();
        let (ok, payload) = as_result(r);
        assert!(ok);
        assert_eq!(as_string(payload), "");
    }

    #[test]
    fn to_string_valid_utf8_multibyte() {
        // "🌟" is 4 bytes in UTF-8: F0 9F 8C 9F
        let r = builtin_bytes_to_string(&[bytes(&[0xF0, 0x9F, 0x8C, 0x9F])]).unwrap();
        let (ok, payload) = as_result(r);
        assert!(ok);
        assert_eq!(as_string(payload), "🌟");
    }

    #[test]
    fn to_string_invalid_utf8_is_err() {
        // 0xFF is never a valid start byte in UTF-8.
        let r = builtin_bytes_to_string(&[bytes(&[0xFF, 0xFE, 0xFD])]).unwrap();
        let (ok, payload) = as_result(r);
        assert!(!ok);
        let msg = as_string(payload);
        assert!(msg.contains("invalid UTF-8"));
    }

    #[test]
    fn to_string_invalid_utf8_in_middle_reports_position() {
        // "ab" + 0xFF + "cd" — UTF-8 valid up to byte 2.
        let r = builtin_bytes_to_string(&[bytes(&[b'a', b'b', 0xFF, b'c', b'd'])]).unwrap();
        let (ok, payload) = as_result(r);
        assert!(!ok);
        let msg = as_string(payload);
        assert!(msg.contains("byte 2"), "got {}", msg);
    }

    #[test]
    fn to_string_rejects_non_bytes() {
        let err = builtin_bytes_to_string(&[Value::String("hello".to_string())]).unwrap_err();
        assert!(err.contains("expected Bytes"));
    }

    // --- general ---

    #[test]
    fn arity_diagnostics_consistent() {
        let err = builtin_bytes_strip_prefix(&[]).unwrap_err();
        assert!(err.contains("expected 2"));
        let err = builtin_bytes_strip_suffix(&[]).unwrap_err();
        assert!(err.contains("expected 2"));
        let err = builtin_bytes_to_string(&[]).unwrap_err();
        assert!(err.contains("expected 1"));
    }

    #[test]
    fn strip_round_trips_with_concat() {
        // For any (prefix, suffix, middle): strip_prefix(concat(prefix, middle), prefix) == middle
        let prefix = vec![0xDE, 0xADu8];
        let middle = vec![0x01, 0x02, 0x03u8];
        let combined: Vec<u8> = prefix
            .iter()
            .copied()
            .chain(middle.iter().copied())
            .collect();
        let r = builtin_bytes_strip_prefix(&[bytes(&combined), bytes(&prefix)]).unwrap();
        assert_eq!(as_bytes(r), middle);
    }
}
