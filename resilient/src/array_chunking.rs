//! RES-1142: array chunking + striding + rotation primitives.
//!
//! Five pure leaf builtins that round out the `Array` slicing surface
//! alongside the existing `array_window` (RES-455) and `array_repeat`
//! (RES-456). `array_window` exposes overlapping sliding windows; this
//! ticket adds the disjoint / strided / cyclic variants:
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `array_chunks(arr, n)`       | `(Array, Int) -> Array` | Non-overlapping chunks; last chunk may be shorter |
//! | `array_chunks_exact(arr, n)` | `(Array, Int) -> Array` | Strict version — drops remainder |
//! | `array_step(arr, n)`         | `(Array, Int) -> Array` | Every nth element starting at index 0 |
//! | `array_rotate_left(arr, n)`  | `(Array, Int) -> Array` | Rotate left by `n` positions (`n` is taken mod `len`) |
//! | `array_rotate_right(arr, n)` | `(Array, Int) -> Array` | Rotate right by `n` positions |
//!
//! All five require `n > 0` and return a fresh `Array`. Input never
//! mutated. `n` larger than `len` is allowed for chunks / step (returns
//! a single full chunk / single-element / empty result) and rotation
//! (taken modulo `len`).

use crate::{RResult, Value};

/// `array_chunks(arr, n) -> Array[Array]` — split `arr` into
/// non-overlapping sub-arrays of length `n`. If `len` is not divisible
/// by `n`, the last chunk is shorter. `n` must be positive.
pub(crate) fn builtin_array_chunks(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::Int(n)] => {
            if *n <= 0 {
                return Err(format!(
                    "array_chunks: chunk size must be positive, got {}",
                    n
                ));
            }
            let size = *n as usize;
            let chunks: Vec<Value> = items
                .chunks(size)
                .map(|c| Value::Array(c.to_vec()))
                .collect();
            Ok(Value::Array(chunks))
        }
        [a, b] => Err(format!(
            "array_chunks: expected (array, int), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "array_chunks: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `array_chunks_exact(arr, n) -> Array[Array]` — like `array_chunks`
/// but drops the final partial chunk when `len % n != 0`. Useful when
/// every chunk must be exactly `n` elements (SIMD lanes, fixed-size
/// frames). `n` must be positive.
pub(crate) fn builtin_array_chunks_exact(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::Int(n)] => {
            if *n <= 0 {
                return Err(format!(
                    "array_chunks_exact: chunk size must be positive, got {}",
                    n
                ));
            }
            let size = *n as usize;
            let chunks: Vec<Value> = items
                .chunks_exact(size)
                .map(|c| Value::Array(c.to_vec()))
                .collect();
            Ok(Value::Array(chunks))
        }
        [a, b] => Err(format!(
            "array_chunks_exact: expected (array, int), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "array_chunks_exact: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `array_step(arr, n) -> Array` — every nth element starting at
/// index 0. `n` must be positive. `n = 1` returns a clone; `n >= len`
/// returns just the first element (or empty if input is empty).
pub(crate) fn builtin_array_step(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::Int(n)] => {
            if *n <= 0 {
                return Err(format!("array_step: stride must be positive, got {}", n));
            }
            let stride = *n as usize;
            let stepped: Vec<Value> = items.iter().step_by(stride).cloned().collect();
            Ok(Value::Array(stepped))
        }
        [a, b] => Err(format!(
            "array_step: expected (array, int), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "array_step: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `array_rotate_left(arr, n) -> Array` — rotate `arr` left by `n`
/// positions. `n` must be non-negative. Effective rotation is taken
/// modulo `len`, so `n >= len` is well-defined. Empty array returns
/// empty regardless of `n`. Element at index `i` ends up at
/// `(i - n + len) mod len`.
pub(crate) fn builtin_array_rotate_left(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::Int(n)] => {
            if *n < 0 {
                return Err(format!(
                    "array_rotate_left: count must be non-negative, got {}",
                    n
                ));
            }
            if items.is_empty() {
                return Ok(Value::Array(Vec::new()));
            }
            let len = items.len();
            let shift = (*n as usize) % len;
            let mut out = Vec::with_capacity(len);
            out.extend_from_slice(&items[shift..]);
            out.extend_from_slice(&items[..shift]);
            Ok(Value::Array(out))
        }
        [a, b] => Err(format!(
            "array_rotate_left: expected (array, int), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "array_rotate_left: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `array_rotate_right(arr, n) -> Array` — rotate `arr` right by `n`
/// positions. Same contract as `array_rotate_left`, in the opposite
/// direction. Element at index `i` ends up at `(i + n) mod len`.
pub(crate) fn builtin_array_rotate_right(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::Int(n)] => {
            if *n < 0 {
                return Err(format!(
                    "array_rotate_right: count must be non-negative, got {}",
                    n
                ));
            }
            if items.is_empty() {
                return Ok(Value::Array(Vec::new()));
            }
            let len = items.len();
            let shift = (*n as usize) % len;
            // Right-by-shift = left-by-(len - shift).
            let split = len - shift;
            let mut out = Vec::with_capacity(len);
            out.extend_from_slice(&items[split..]);
            out.extend_from_slice(&items[..split]);
            Ok(Value::Array(out))
        }
        [a, b] => Err(format!(
            "array_rotate_right: expected (array, int), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "array_rotate_right: expected 2 arguments, got {}",
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

    fn as_ints(v: Value) -> Vec<i64> {
        match v {
            Value::Array(items) => items
                .into_iter()
                .map(|x| match x {
                    Value::Int(n) => n,
                    other => panic!("expected Int, got {:?}", other),
                })
                .collect(),
            other => panic!("expected Array, got {:?}", other),
        }
    }

    fn as_nested_ints(v: Value) -> Vec<Vec<i64>> {
        match v {
            Value::Array(outer) => outer.into_iter().map(as_ints).collect(),
            other => panic!("expected Array of Array, got {:?}", other),
        }
    }

    #[test]
    fn chunks_evenly_divisible() {
        let r = builtin_array_chunks(&[ints(&[1, 2, 3, 4, 5, 6]), Value::Int(2)]).unwrap();
        assert_eq!(as_nested_ints(r), vec![vec![1, 2], vec![3, 4], vec![5, 6]]);
    }

    #[test]
    fn chunks_with_remainder() {
        let r = builtin_array_chunks(&[ints(&[1, 2, 3, 4, 5]), Value::Int(2)]).unwrap();
        assert_eq!(as_nested_ints(r), vec![vec![1, 2], vec![3, 4], vec![5]]);
    }

    #[test]
    fn chunks_n_greater_than_len_returns_one_full_chunk() {
        let r = builtin_array_chunks(&[ints(&[1, 2, 3]), Value::Int(10)]).unwrap();
        assert_eq!(as_nested_ints(r), vec![vec![1, 2, 3]]);
    }

    #[test]
    fn chunks_empty_array_is_empty() {
        let r = builtin_array_chunks(&[ints(&[]), Value::Int(3)]).unwrap();
        assert_eq!(as_nested_ints(r), Vec::<Vec<i64>>::new());
    }

    #[test]
    fn chunks_n_one_returns_singletons() {
        let r = builtin_array_chunks(&[ints(&[1, 2, 3]), Value::Int(1)]).unwrap();
        assert_eq!(as_nested_ints(r), vec![vec![1], vec![2], vec![3]]);
    }

    #[test]
    fn chunks_rejects_zero_and_negative() {
        let err = builtin_array_chunks(&[ints(&[1, 2]), Value::Int(0)]).unwrap_err();
        assert!(err.contains("must be positive"));
        let err = builtin_array_chunks(&[ints(&[1, 2]), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("must be positive"));
    }

    #[test]
    fn chunks_exact_drops_remainder() {
        let r = builtin_array_chunks_exact(&[ints(&[1, 2, 3, 4, 5]), Value::Int(2)]).unwrap();
        assert_eq!(as_nested_ints(r), vec![vec![1, 2], vec![3, 4]]);
    }

    #[test]
    fn chunks_exact_even_split_matches_chunks() {
        let r = builtin_array_chunks_exact(&[ints(&[1, 2, 3, 4, 5, 6]), Value::Int(3)]).unwrap();
        assert_eq!(as_nested_ints(r), vec![vec![1, 2, 3], vec![4, 5, 6]]);
    }

    #[test]
    fn chunks_exact_n_greater_than_len_is_empty() {
        let r = builtin_array_chunks_exact(&[ints(&[1, 2, 3]), Value::Int(10)]).unwrap();
        assert_eq!(as_nested_ints(r), Vec::<Vec<i64>>::new());
    }

    #[test]
    fn step_basic() {
        let r = builtin_array_step(&[ints(&[1, 2, 3, 4, 5, 6, 7]), Value::Int(2)]).unwrap();
        assert_eq!(as_ints(r), vec![1, 3, 5, 7]);
        let r = builtin_array_step(&[ints(&[1, 2, 3, 4, 5, 6, 7]), Value::Int(3)]).unwrap();
        assert_eq!(as_ints(r), vec![1, 4, 7]);
    }

    #[test]
    fn step_one_is_identity() {
        let r = builtin_array_step(&[ints(&[1, 2, 3]), Value::Int(1)]).unwrap();
        assert_eq!(as_ints(r), vec![1, 2, 3]);
    }

    #[test]
    fn step_larger_than_len_returns_first_element() {
        let r = builtin_array_step(&[ints(&[1, 2, 3]), Value::Int(10)]).unwrap();
        assert_eq!(as_ints(r), vec![1]);
    }

    #[test]
    fn step_empty_yields_empty() {
        let r = builtin_array_step(&[ints(&[]), Value::Int(5)]).unwrap();
        assert_eq!(as_ints(r), Vec::<i64>::new());
    }

    #[test]
    fn step_rejects_zero_and_negative() {
        let err = builtin_array_step(&[ints(&[1, 2]), Value::Int(0)]).unwrap_err();
        assert!(err.contains("must be positive"));
        let err = builtin_array_step(&[ints(&[1, 2]), Value::Int(-3)]).unwrap_err();
        assert!(err.contains("must be positive"));
    }

    #[test]
    fn rotate_left_basic() {
        let r = builtin_array_rotate_left(&[ints(&[1, 2, 3, 4, 5]), Value::Int(2)]).unwrap();
        assert_eq!(as_ints(r), vec![3, 4, 5, 1, 2]);
    }

    #[test]
    fn rotate_left_by_zero_is_identity() {
        let r = builtin_array_rotate_left(&[ints(&[1, 2, 3]), Value::Int(0)]).unwrap();
        assert_eq!(as_ints(r), vec![1, 2, 3]);
    }

    #[test]
    fn rotate_left_by_len_is_identity() {
        let r = builtin_array_rotate_left(&[ints(&[1, 2, 3]), Value::Int(3)]).unwrap();
        assert_eq!(as_ints(r), vec![1, 2, 3]);
    }

    #[test]
    fn rotate_left_modulo_len() {
        let r = builtin_array_rotate_left(&[ints(&[1, 2, 3, 4, 5]), Value::Int(7)]).unwrap();
        // 7 mod 5 = 2 → same as rotate-left-by-2
        assert_eq!(as_ints(r), vec![3, 4, 5, 1, 2]);
    }

    #[test]
    fn rotate_left_empty_yields_empty() {
        let r = builtin_array_rotate_left(&[ints(&[]), Value::Int(7)]).unwrap();
        assert_eq!(as_ints(r), Vec::<i64>::new());
    }

    #[test]
    fn rotate_left_rejects_negative() {
        let err = builtin_array_rotate_left(&[ints(&[1]), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("non-negative"));
    }

    #[test]
    fn rotate_right_basic() {
        let r = builtin_array_rotate_right(&[ints(&[1, 2, 3, 4, 5]), Value::Int(2)]).unwrap();
        assert_eq!(as_ints(r), vec![4, 5, 1, 2, 3]);
    }

    #[test]
    fn rotate_right_modulo_len() {
        let r = builtin_array_rotate_right(&[ints(&[1, 2, 3, 4, 5]), Value::Int(12)]).unwrap();
        // 12 mod 5 = 2
        assert_eq!(as_ints(r), vec![4, 5, 1, 2, 3]);
    }

    #[test]
    fn rotate_left_then_right_is_identity() {
        let original = ints(&[1, 2, 3, 4, 5, 6, 7]);
        let left = builtin_array_rotate_left(&[original.clone(), Value::Int(3)]).unwrap();
        let back = builtin_array_rotate_right(&[left, Value::Int(3)]).unwrap();
        assert_eq!(as_ints(back), as_ints(original));
    }

    #[test]
    fn rotate_right_rejects_negative() {
        let err = builtin_array_rotate_right(&[ints(&[1]), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("non-negative"));
    }

    #[test]
    fn arity_diagnostics_consistent() {
        for f in [
            builtin_array_chunks,
            builtin_array_chunks_exact,
            builtin_array_step,
            builtin_array_rotate_left,
            builtin_array_rotate_right,
        ] {
            let err = f(&[ints(&[1])]).unwrap_err();
            assert!(err.contains("expected 2"), "got {}", err);
            let err = f(&[Value::Int(5), Value::Int(1)]).unwrap_err();
            assert!(err.contains("expected (array, int)"), "got {}", err);
        }
    }
}
