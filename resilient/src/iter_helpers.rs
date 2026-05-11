//! RES-1164: small iteration helpers.
//!
//! Three pure leaf builtins:
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `enumerate(arr)`         | `(Array) -> Array`                | `[[i, v], ...]` index-value pairs |
//! | `array_zip3(a, b, c)`    | `(Array, Array, Array) -> Array`  | 3-way zip; length = min |
//! | `string_truncate(s, n)`  | `(String, Int) -> String`         | First `n` Unicode scalars |

use crate::{RResult, Value};

/// `enumerate(arr) -> Array` — `[[0, arr[0]], [1, arr[1]], ...]`.
/// Indices start at 0. Empty input returns empty.
pub(crate) fn builtin_enumerate(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            let out: Vec<Value> = items
                .iter()
                .enumerate()
                .map(|(i, v)| Value::Array(vec![Value::Int(i as i64), v.clone()]))
                .collect();
            Ok(Value::Array(out))
        }
        [other] => Err(format!("enumerate: expected array, got {}", other)),
        _ => Err(format!(
            "enumerate: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_zip3(a, b, c) -> Array[[A, B, C]]` — three-way zip. Length is
/// `min(|a|, |b|, |c|)`; trailing elements of longer inputs are dropped.
pub(crate) fn builtin_array_zip3(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(a), Value::Array(b), Value::Array(c)] => {
            let n = a.len().min(b.len()).min(c.len());
            let mut out: Vec<Value> = Vec::with_capacity(n);
            for i in 0..n {
                out.push(Value::Array(vec![a[i].clone(), b[i].clone(), c[i].clone()]));
            }
            Ok(Value::Array(out))
        }
        [a, b, c] => Err(format!(
            "array_zip3: expected (array, array, array), got ({}, {}, {})",
            a, b, c
        )),
        _ => Err(format!(
            "array_zip3: expected 3 arguments, got {}",
            args.len()
        )),
    }
}

/// `string_truncate(s, n) -> String` — keep the first `n` Unicode scalars.
/// `n` must be non-negative. `n` larger than the char count returns `s`
/// unchanged.
pub(crate) fn builtin_string_truncate(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s), Value::Int(n)] => {
            if *n < 0 {
                return Err(format!(
                    "string_truncate: n must be non-negative, got {}",
                    n
                ));
            }
            let max = *n as usize;
            // Iterate chars and rebuild — Unicode-scalar aware so we
            // don't slice mid-codepoint.
            let out: String = s.chars().take(max).collect();
            Ok(Value::String(out))
        }
        [a, b] => Err(format!(
            "string_truncate: expected (string, int), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "string_truncate: expected 2 arguments, got {}",
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

    fn as_array(v: Value) -> Vec<Value> {
        match v {
            Value::Array(items) => items,
            other => panic!("expected Array, got {:?}", other),
        }
    }

    fn as_int(v: Value) -> i64 {
        match v {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    fn as_string(v: Value) -> String {
        match v {
            Value::String(s) => s,
            other => panic!("expected String, got {:?}", other),
        }
    }

    // --- enumerate ---

    #[test]
    fn enumerate_basic() {
        let r = builtin_enumerate(&[ints(&[10, 20, 30])]).unwrap();
        let outer = as_array(r);
        assert_eq!(outer.len(), 3);
        for (expected_idx, pair) in outer.into_iter().enumerate() {
            let p = as_array(pair);
            assert_eq!(p.len(), 2);
            assert_eq!(as_int(p[0].clone()), expected_idx as i64);
            assert_eq!(as_int(p[1].clone()), (expected_idx as i64 + 1) * 10);
        }
    }

    #[test]
    fn enumerate_empty() {
        let r = builtin_enumerate(&[ints(&[])]).unwrap();
        assert_eq!(as_array(r).len(), 0);
    }

    #[test]
    fn enumerate_singleton() {
        let r = builtin_enumerate(&[ints(&[42])]).unwrap();
        let outer = as_array(r);
        assert_eq!(outer.len(), 1);
        let p = as_array(outer.into_iter().next().unwrap());
        assert_eq!(as_int(p[0].clone()), 0);
        assert_eq!(as_int(p[1].clone()), 42);
    }

    #[test]
    fn enumerate_mixed_types() {
        let arr = Value::Array(vec![
            Value::Int(1),
            Value::String("hello".to_string()),
            Value::Bool(true),
        ]);
        let r = as_array(builtin_enumerate(&[arr]).unwrap());
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn enumerate_rejects_non_array() {
        let err = builtin_enumerate(&[Value::Int(7)]).unwrap_err();
        assert!(err.contains("expected array"));
    }

    // --- array_zip3 ---

    #[test]
    fn zip3_basic() {
        let r = builtin_array_zip3(&[
            ints(&[1, 2, 3]),
            ints(&[10, 20, 30]),
            ints(&[100, 200, 300]),
        ])
        .unwrap();
        let outer = as_array(r);
        assert_eq!(outer.len(), 3);
        let first = as_array(outer.into_iter().next().unwrap());
        assert_eq!(first.len(), 3);
        assert_eq!(as_int(first[0].clone()), 1);
        assert_eq!(as_int(first[1].clone()), 10);
        assert_eq!(as_int(first[2].clone()), 100);
    }

    #[test]
    fn zip3_truncates_to_shortest() {
        let r = builtin_array_zip3(&[ints(&[1, 2, 3, 4]), ints(&[10, 20]), ints(&[100, 200, 300])])
            .unwrap();
        let outer = as_array(r);
        assert_eq!(outer.len(), 2); // min(4, 2, 3) = 2
    }

    #[test]
    fn zip3_one_empty_input_is_empty() {
        let r = builtin_array_zip3(&[ints(&[1, 2]), ints(&[]), ints(&[10, 20])]).unwrap();
        assert_eq!(as_array(r).len(), 0);
    }

    #[test]
    fn zip3_all_empty_is_empty() {
        let r = builtin_array_zip3(&[ints(&[]), ints(&[]), ints(&[])]).unwrap();
        assert_eq!(as_array(r).len(), 0);
    }

    #[test]
    fn zip3_rejects_non_array() {
        let err = builtin_array_zip3(&[ints(&[1]), Value::Int(5), ints(&[2])]).unwrap_err();
        assert!(err.contains("expected (array, array, array)"));
    }

    #[test]
    fn zip3_rejects_wrong_arity() {
        let err = builtin_array_zip3(&[ints(&[1])]).unwrap_err();
        assert!(err.contains("expected 3"));
        let err = builtin_array_zip3(&[ints(&[1]), ints(&[2])]).unwrap_err();
        assert!(err.contains("expected 3"));
    }

    // --- string_truncate ---

    #[test]
    fn truncate_basic() {
        let s = Value::String("hello".to_string());
        let r = builtin_string_truncate(&[s, Value::Int(3)]).unwrap();
        assert_eq!(as_string(r), "hel");
    }

    #[test]
    fn truncate_exact_length() {
        let s = Value::String("hello".to_string());
        let r = builtin_string_truncate(&[s, Value::Int(5)]).unwrap();
        assert_eq!(as_string(r), "hello");
    }

    #[test]
    fn truncate_larger_than_length() {
        let s = Value::String("hello".to_string());
        let r = builtin_string_truncate(&[s, Value::Int(100)]).unwrap();
        assert_eq!(as_string(r), "hello");
    }

    #[test]
    fn truncate_to_zero_is_empty() {
        let s = Value::String("hello".to_string());
        let r = builtin_string_truncate(&[s, Value::Int(0)]).unwrap();
        assert_eq!(as_string(r), "");
    }

    #[test]
    fn truncate_empty_string() {
        let s = Value::String("".to_string());
        let r = builtin_string_truncate(&[s, Value::Int(5)]).unwrap();
        assert_eq!(as_string(r), "");
    }

    #[test]
    fn truncate_unicode_aware() {
        // "café" is 4 chars but 5 bytes — truncate should be char-aware
        // and not split the é codepoint.
        let s = Value::String("café".to_string());
        let r = builtin_string_truncate(&[s, Value::Int(3)]).unwrap();
        assert_eq!(as_string(r), "caf");
    }

    #[test]
    fn truncate_multi_byte_at_boundary() {
        // 🌟 is a 4-byte multi-byte codepoint. Truncating to 1 char
        // should keep it intact (not slice into the middle).
        let s = Value::String("🌟hello".to_string());
        let r = builtin_string_truncate(&[s, Value::Int(1)]).unwrap();
        assert_eq!(as_string(r), "🌟");
    }

    #[test]
    fn truncate_rejects_negative() {
        let s = Value::String("hello".to_string());
        let err = builtin_string_truncate(&[s, Value::Int(-1)]).unwrap_err();
        assert!(err.contains("non-negative"));
    }

    #[test]
    fn truncate_rejects_wrong_types() {
        let err = builtin_string_truncate(&[Value::Int(0), Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected (string, int)"));
    }

    // --- general ---

    #[test]
    fn arity_diagnostics_consistent() {
        let err = builtin_enumerate(&[]).unwrap_err();
        assert!(err.contains("expected 1"));
        let err = builtin_string_truncate(&[]).unwrap_err();
        assert!(err.contains("expected 2"));
    }
}
