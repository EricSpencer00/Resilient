//! RES-1148: `array_binary_search`, `_float`, `_string` — sorted-array
//! lookup primitives that return `Result<Int, Int>`.
//!
//! Companions to RES-1146's float / string sort + `is_sorted` predicates.
//! Without binary search, sorted arrays don't get their main benefit —
//! every membership query falls back to O(N) `array_index_of`.
//!
//! | Builtin | Signature | Found / Not-found |
//! |---|---|---|
//! | `array_binary_search(arr, target)`        | `(Array, Int) -> Result<Int, Int>`    | `Ok(idx)` / `Err(insertion_pos)` |
//! | `array_binary_search_float(arr, target)`  | `(Array, Float) -> Result<Int, Int>`  | NaN-safe via IEEE 754 total order |
//! | `array_binary_search_string(arr, target)` | `(Array, String) -> Result<Int, Int>` | Lex byte-wise |
//!
//! Caller must pass a sorted array — the function does not verify
//! sortedness, matching `slice::binary_search`. Unsorted input
//! produces an unspecified `Ok` / `Err` index but never a panic.

use crate::{RResult, Value};

fn result_int(ok: bool, idx: usize) -> Value {
    Value::Result {
        ok,
        payload: Box::new(Value::Int(idx as i64)),
    }
}

/// `array_binary_search(arr, target) -> Result<Int, Int>` — lookup in a
/// sorted int array. Array elements must all be ints; mixed types are
/// rejected with a typed error.
///
/// RES-2034: pre-walks `items` for type validation, then dispatches
/// `binary_search_by` directly on the original `&[Value]`. The previous
/// implementation materialized a throwaway `Vec<i64>` of length N just
/// to satisfy `binary_search`'s signature.
pub(crate) fn builtin_array_binary_search(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::Int(target)] => {
            for v in items {
                if !matches!(v, Value::Int(_)) {
                    return Err(format!(
                        "array_binary_search: expected all int elements, got {}",
                        v
                    ));
                }
            }
            let result = items.binary_search_by(|v| match v {
                Value::Int(n) => n.cmp(target),
                _ => unreachable!("validated above"),
            });
            match result {
                Ok(idx) => Ok(result_int(true, idx)),
                Err(idx) => Ok(result_int(false, idx)),
            }
        }
        [Value::Array(_), other] => Err(format!(
            "array_binary_search: target must be Int, got {}",
            other
        )),
        [other, _] => Err(format!(
            "array_binary_search: first argument must be array, got {}",
            other
        )),
        _ => Err(format!(
            "array_binary_search: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `array_binary_search_float(arr, target) -> Result<Int, Int>` —
/// lookup in a sorted float array. Comparison uses `f64::total_cmp` so
/// the search behaves correctly on NaN / `±0` / signed-NaN inputs,
/// matching `array_sort_float`'s ordering.
///
/// RES-2034: pre-walks `items` for type validation, then dispatches
/// `binary_search_by` directly on the original `&[Value]` (drops the
/// previous `Vec<f64>` materialization).
pub(crate) fn builtin_array_binary_search_float(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::Float(target)] => {
            for v in items {
                if !matches!(v, Value::Float(_)) {
                    return Err(format!(
                        "array_binary_search_float: expected all float elements, got {}",
                        v
                    ));
                }
            }
            let result = items.binary_search_by(|v| match v {
                Value::Float(f) => f.total_cmp(target),
                _ => unreachable!("validated above"),
            });
            match result {
                Ok(idx) => Ok(result_int(true, idx)),
                Err(idx) => Ok(result_int(false, idx)),
            }
        }
        [Value::Array(_), other] => Err(format!(
            "array_binary_search_float: target must be Float, got {}",
            other
        )),
        [other, _] => Err(format!(
            "array_binary_search_float: first argument must be array, got {}",
            other
        )),
        _ => Err(format!(
            "array_binary_search_float: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `array_binary_search_string(arr, target) -> Result<Int, Int>` —
/// lookup in a sorted string array. Comparison is byte-wise on the
/// UTF-8 representation, matching `array_sort_string`'s ordering.
///
/// RES-2034: pre-walks `items` for type validation, then dispatches
/// `binary_search_by` directly on the original `&[Value]`. The previous
/// `Vec<&String>` materialization (one pointer per element) is gone.
pub(crate) fn builtin_array_binary_search_string(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::String(target)] => {
            for v in items {
                if !matches!(v, Value::String(_)) {
                    return Err(format!(
                        "array_binary_search_string: expected all string elements, got {}",
                        v
                    ));
                }
            }
            let target_str = target.as_str();
            let result = items.binary_search_by(|v| match v {
                Value::String(s) => s.as_str().cmp(target_str),
                _ => unreachable!("validated above"),
            });
            match result {
                Ok(idx) => Ok(result_int(true, idx)),
                Err(idx) => Ok(result_int(false, idx)),
            }
        }
        [Value::Array(_), other] => Err(format!(
            "array_binary_search_string: target must be String, got {}",
            other
        )),
        [other, _] => Err(format!(
            "array_binary_search_string: first argument must be array, got {}",
            other
        )),
        _ => Err(format!(
            "array_binary_search_string: expected 2 arguments, got {}",
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

    fn floats(xs: &[f64]) -> Value {
        Value::Array(xs.iter().map(|&f| Value::Float(f)).collect())
    }

    fn strings(xs: &[&str]) -> Value {
        Value::Array(xs.iter().map(|s| Value::String(s.to_string())).collect())
    }

    fn as_result(v: Value) -> (bool, i64) {
        match v {
            Value::Result { ok, payload } => match *payload {
                Value::Int(n) => (ok, n),
                other => panic!("expected Int payload, got {:?}", other),
            },
            other => panic!("expected Result, got {:?}", other),
        }
    }

    // --- int ---

    #[test]
    fn binary_search_int_found() {
        let arr = ints(&[1, 3, 5, 7, 9]);
        assert_eq!(
            as_result(builtin_array_binary_search(&[arr.clone(), Value::Int(1)]).unwrap()),
            (true, 0)
        );
        assert_eq!(
            as_result(builtin_array_binary_search(&[arr.clone(), Value::Int(5)]).unwrap()),
            (true, 2)
        );
        assert_eq!(
            as_result(builtin_array_binary_search(&[arr, Value::Int(9)]).unwrap()),
            (true, 4)
        );
    }

    #[test]
    fn binary_search_int_not_found_returns_insertion_index() {
        let arr = ints(&[1, 3, 5, 7, 9]);
        // 0 would be inserted at the start.
        assert_eq!(
            as_result(builtin_array_binary_search(&[arr.clone(), Value::Int(0)]).unwrap()),
            (false, 0)
        );
        // 4 would be inserted at index 2 (between 3 and 5).
        assert_eq!(
            as_result(builtin_array_binary_search(&[arr.clone(), Value::Int(4)]).unwrap()),
            (false, 2)
        );
        // 10 would be appended at the end.
        assert_eq!(
            as_result(builtin_array_binary_search(&[arr, Value::Int(10)]).unwrap()),
            (false, 5)
        );
    }

    #[test]
    fn binary_search_int_empty_returns_err_zero() {
        let r = builtin_array_binary_search(&[ints(&[]), Value::Int(42)]).unwrap();
        assert_eq!(as_result(r), (false, 0));
    }

    #[test]
    fn binary_search_int_single_element() {
        let arr = ints(&[5]);
        assert_eq!(
            as_result(builtin_array_binary_search(&[arr.clone(), Value::Int(5)]).unwrap()),
            (true, 0)
        );
        assert_eq!(
            as_result(builtin_array_binary_search(&[arr.clone(), Value::Int(3)]).unwrap()),
            (false, 0)
        );
        assert_eq!(
            as_result(builtin_array_binary_search(&[arr, Value::Int(7)]).unwrap()),
            (false, 1)
        );
    }

    #[test]
    fn binary_search_int_rejects_non_int_target() {
        let err = builtin_array_binary_search(&[ints(&[1, 2, 3]), Value::Float(2.0)]).unwrap_err();
        assert!(err.contains("target must be Int"));
    }

    #[test]
    fn binary_search_int_rejects_non_int_elements() {
        let err = builtin_array_binary_search(&[
            Value::Array(vec![Value::Int(1), Value::Float(2.0)]),
            Value::Int(1),
        ])
        .unwrap_err();
        assert!(err.contains("expected all int elements"));
    }

    // --- float ---

    #[test]
    fn binary_search_float_found() {
        let arr = floats(&[1.0, 1.5, 2.0, 2.5, 3.0]);
        let r = builtin_array_binary_search_float(&[arr, Value::Float(2.5)]).unwrap();
        assert_eq!(as_result(r), (true, 3));
    }

    #[test]
    fn binary_search_float_handles_total_order() {
        // -0.0 < +0.0 under total order — they're distinct positions.
        let arr = floats(&[-1.0, -0.0, 0.0, 1.0]);
        let r = builtin_array_binary_search_float(&[arr.clone(), Value::Float(-0.0)]).unwrap();
        assert_eq!(as_result(r), (true, 1));
        let r = builtin_array_binary_search_float(&[arr, Value::Float(0.0)]).unwrap();
        assert_eq!(as_result(r), (true, 2));
    }

    #[test]
    fn binary_search_float_nan_search_in_nan_terminated_array() {
        // Under total order NaN sorts after +inf.
        let arr = floats(&[1.0, 2.0, f64::INFINITY, f64::NAN]);
        let r = builtin_array_binary_search_float(&[arr, Value::Float(f64::NAN)]).unwrap();
        assert_eq!(as_result(r), (true, 3));
    }

    #[test]
    fn binary_search_float_not_found() {
        let arr = floats(&[1.0, 2.0, 3.0]);
        let r = builtin_array_binary_search_float(&[arr, Value::Float(2.5)]).unwrap();
        assert_eq!(as_result(r), (false, 2));
    }

    #[test]
    fn binary_search_float_rejects_non_float_target() {
        let err = builtin_array_binary_search_float(&[floats(&[1.0]), Value::Int(1)]).unwrap_err();
        assert!(err.contains("target must be Float"));
    }

    // --- string ---

    #[test]
    fn binary_search_string_found() {
        let arr = strings(&["alice", "bob", "carol", "dave"]);
        assert_eq!(
            as_result(
                builtin_array_binary_search_string(&[
                    arr.clone(),
                    Value::String("bob".to_string())
                ])
                .unwrap()
            ),
            (true, 1)
        );
        assert_eq!(
            as_result(
                builtin_array_binary_search_string(&[arr, Value::String("dave".to_string())])
                    .unwrap()
            ),
            (true, 3)
        );
    }

    #[test]
    fn binary_search_string_not_found() {
        let arr = strings(&["alice", "carol", "eve"]);
        // "bob" sorts between "alice" and "carol".
        let r =
            builtin_array_binary_search_string(&[arr.clone(), Value::String("bob".to_string())])
                .unwrap();
        assert_eq!(as_result(r), (false, 1));
        // "aardvark" sorts before "alice".
        let r = builtin_array_binary_search_string(&[arr, Value::String("aardvark".to_string())])
            .unwrap();
        assert_eq!(as_result(r), (false, 0));
    }

    #[test]
    fn binary_search_string_empty() {
        let r = builtin_array_binary_search_string(&[strings(&[]), Value::String("x".to_string())])
            .unwrap();
        assert_eq!(as_result(r), (false, 0));
    }

    #[test]
    fn binary_search_string_rejects_non_string_target() {
        let err =
            builtin_array_binary_search_string(&[strings(&["a", "b"]), Value::Int(1)]).unwrap_err();
        assert!(err.contains("target must be String"));
    }

    // --- general ---

    #[test]
    fn arity_diagnostics_consistent() {
        for f in [
            builtin_array_binary_search,
            builtin_array_binary_search_float,
            builtin_array_binary_search_string,
        ] {
            let err = f(&[Value::Array(vec![])]).unwrap_err();
            assert!(err.contains("expected 2"), "got {}", err);
        }
        let err = builtin_array_binary_search(&[Value::Int(5), Value::Int(1)]).unwrap_err();
        assert!(err.contains("first argument must be array"));
    }

    #[test]
    fn insertion_index_property() {
        // For any miss, inserting at the returned index keeps the array sorted.
        let arr = vec![1i64, 3, 5, 7, 9];
        for target in [0, 2, 4, 6, 8, 10] {
            let r = builtin_array_binary_search(&[ints(&arr), Value::Int(target)]).unwrap();
            let (ok, idx) = as_result(r);
            assert!(!ok, "expected Err for absent target {}", target);
            // Simulate insertion.
            let mut inserted = arr.clone();
            inserted.insert(idx as usize, target);
            assert!(
                inserted.windows(2).all(|w| w[0] <= w[1]),
                "inserting {} at {} broke sort: {:?}",
                target,
                idx,
                inserted
            );
        }
    }
}
