//! RES-1158: array set-style helpers + fallback-safe first/last + index_of_last.
//!
//! Five pure leaf builtins that round out the array surface:
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `array_difference(a, b)`        | `(Array, Array) -> Array` | Elements of `a` not in `b` |
//! | `array_intersection(a, b)`      | `(Array, Array) -> Array` | Elements of `a` that are in `b` |
//! | `array_index_of_last(arr, x)`   | `(Array, T) -> Int`        | Last index where element == x, or -1 |
//! | `array_first_or(arr, default)`  | `(Array, T) -> T`          | First element or default if empty |
//! | `array_last_or(arr, default)`   | `(Array, T) -> T`          | Last element or default if empty |
//!
//! `array_difference` / `array_intersection` preserve `a`'s order and
//! duplicates. Same equality semantics as the existing `array_contains` /
//! `array_index_of` family (scalar Int / Float / String / Bool; nested
//! values raise a typed error).

use crate::{RResult, Value};

/// Membership test using the existing `array_search_eq` /
/// `array_member_of` helpers in `lib.rs`. Mirrors `array_union`'s
/// equality semantics.
fn member_of(name: &str, v: &Value, set: &[Value]) -> RResult<bool> {
    crate::array_member_of(name, v, set)
}

/// `array_difference(a, b) -> Array` — elements of `a` that do not
/// appear in `b`. Preserves `a`'s order. Duplicates in `a` are kept
/// iff they don't appear in `b`.
pub(crate) fn builtin_array_difference(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(a), Value::Array(b)] => {
            let mut out: Vec<Value> = Vec::with_capacity(a.len());
            for v in a {
                if !member_of("array_difference", v, b)? {
                    out.push(v.clone());
                }
            }
            Ok(Value::Array(out))
        }
        [a, b] => Err(format!(
            "array_difference: expected (array, array), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "array_difference: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `array_intersection(a, b) -> Array` — elements of `a` that also
/// appear in `b`. Preserves `a`'s order and duplicates.
pub(crate) fn builtin_array_intersection(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(a), Value::Array(b)] => {
            let mut out: Vec<Value> = Vec::new();
            for v in a {
                if member_of("array_intersection", v, b)? {
                    out.push(v.clone());
                }
            }
            Ok(Value::Array(out))
        }
        [a, b] => Err(format!(
            "array_intersection: expected (array, array), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "array_intersection: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `array_index_of_last(arr, x) -> Int` — highest index where
/// `arr[i] == x`, or `-1` if not found. Same equality semantics as
/// `array_index_of`.
pub(crate) fn builtin_array_index_of_last(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), needle] => {
            for (i, v) in items.iter().enumerate().rev() {
                match crate::array_search_eq(v, needle) {
                    Some(true) => return Ok(Value::Int(i as i64)),
                    Some(false) => {}
                    None => {
                        return Err(format!(
                            "array_index_of_last: element types not comparable ({} vs {})",
                            v, needle
                        ));
                    }
                }
            }
            Ok(Value::Int(-1))
        }
        [a, _] => Err(format!(
            "array_index_of_last: first argument must be array, got {}",
            a
        )),
        _ => Err(format!(
            "array_index_of_last: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `array_first_or(arr, default) -> T` — first element if non-empty,
/// else `default`. Never errors on empty input.
pub(crate) fn builtin_array_first_or(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), default] => match items.first() {
            Some(v) => Ok(v.clone()),
            None => Ok(default.clone()),
        },
        [a, _] => Err(format!(
            "array_first_or: first argument must be array, got {}",
            a
        )),
        _ => Err(format!(
            "array_first_or: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `array_last_or(arr, default) -> T` — last element if non-empty,
/// else `default`. Never errors on empty input.
pub(crate) fn builtin_array_last_or(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), default] => match items.last() {
            Some(v) => Ok(v.clone()),
            None => Ok(default.clone()),
        },
        [a, _] => Err(format!(
            "array_last_or: first argument must be array, got {}",
            a
        )),
        _ => Err(format!(
            "array_last_or: expected 2 arguments, got {}",
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

    fn as_int(v: Value) -> i64 {
        match v {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    fn as_int_vec(v: Value) -> Vec<i64> {
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

    fn as_string(v: Value) -> String {
        match v {
            Value::String(s) => s,
            other => panic!("expected String, got {:?}", other),
        }
    }

    // --- array_difference ---

    #[test]
    fn difference_basic() {
        let r = builtin_array_difference(&[ints(&[1, 2, 3, 4]), ints(&[2, 4])]).unwrap();
        assert_eq!(as_int_vec(r), vec![1, 3]);
    }

    #[test]
    fn difference_preserves_order_and_duplicates() {
        let r = builtin_array_difference(&[ints(&[1, 2, 1, 3, 1, 4]), ints(&[3])]).unwrap();
        assert_eq!(as_int_vec(r), vec![1, 2, 1, 1, 4]);
    }

    #[test]
    fn difference_with_empty_b_returns_a() {
        let r = builtin_array_difference(&[ints(&[1, 2, 3]), ints(&[])]).unwrap();
        assert_eq!(as_int_vec(r), vec![1, 2, 3]);
    }

    #[test]
    fn difference_with_empty_a_is_empty() {
        let r = builtin_array_difference(&[ints(&[]), ints(&[1, 2])]).unwrap();
        assert_eq!(as_int_vec(r), Vec::<i64>::new());
    }

    #[test]
    fn difference_with_all_in_b_is_empty() {
        let r = builtin_array_difference(&[ints(&[1, 2, 3]), ints(&[3, 2, 1])]).unwrap();
        assert_eq!(as_int_vec(r), Vec::<i64>::new());
    }

    // --- array_intersection ---

    #[test]
    fn intersection_basic() {
        let r = builtin_array_intersection(&[ints(&[1, 2, 3, 4]), ints(&[2, 4, 6])]).unwrap();
        assert_eq!(as_int_vec(r), vec![2, 4]);
    }

    #[test]
    fn intersection_preserves_duplicates_in_a() {
        let r = builtin_array_intersection(&[ints(&[1, 2, 1, 3, 1]), ints(&[1])]).unwrap();
        assert_eq!(as_int_vec(r), vec![1, 1, 1]);
    }

    #[test]
    fn intersection_empty_when_disjoint() {
        let r = builtin_array_intersection(&[ints(&[1, 2, 3]), ints(&[4, 5, 6])]).unwrap();
        assert_eq!(as_int_vec(r), Vec::<i64>::new());
    }

    #[test]
    fn intersection_with_empty_inputs() {
        let r = builtin_array_intersection(&[ints(&[]), ints(&[1, 2])]).unwrap();
        assert_eq!(as_int_vec(r), Vec::<i64>::new());
        let r = builtin_array_intersection(&[ints(&[1, 2]), ints(&[])]).unwrap();
        assert_eq!(as_int_vec(r), Vec::<i64>::new());
    }

    // --- array_index_of_last ---

    #[test]
    fn index_of_last_basic() {
        let arr = ints(&[1, 2, 3, 2, 1]);
        assert_eq!(
            as_int(builtin_array_index_of_last(&[arr.clone(), Value::Int(1)]).unwrap()),
            4
        );
        assert_eq!(
            as_int(builtin_array_index_of_last(&[arr.clone(), Value::Int(2)]).unwrap()),
            3
        );
        assert_eq!(
            as_int(builtin_array_index_of_last(&[arr, Value::Int(3)]).unwrap()),
            2
        );
    }

    #[test]
    fn index_of_last_returns_minus_one_when_absent() {
        let r = builtin_array_index_of_last(&[ints(&[1, 2, 3]), Value::Int(99)]).unwrap();
        assert_eq!(as_int(r), -1);
    }

    #[test]
    fn index_of_last_on_empty_array() {
        let r = builtin_array_index_of_last(&[ints(&[]), Value::Int(1)]).unwrap();
        assert_eq!(as_int(r), -1);
    }

    #[test]
    fn index_of_last_single_occurrence() {
        let r = builtin_array_index_of_last(&[ints(&[10, 20, 30]), Value::Int(20)]).unwrap();
        assert_eq!(as_int(r), 1);
    }

    #[test]
    fn index_of_last_string_elements() {
        let arr = Value::Array(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
            Value::String("a".to_string()),
            Value::String("c".to_string()),
        ]);
        let r = builtin_array_index_of_last(&[arr, Value::String("a".to_string())]).unwrap();
        assert_eq!(as_int(r), 2);
    }

    // --- array_first_or ---

    #[test]
    fn first_or_returns_first_when_present() {
        let r = builtin_array_first_or(&[ints(&[10, 20, 30]), Value::Int(-1)]).unwrap();
        assert_eq!(as_int(r), 10);
    }

    #[test]
    fn first_or_returns_default_when_empty() {
        let r = builtin_array_first_or(&[ints(&[]), Value::Int(-1)]).unwrap();
        assert_eq!(as_int(r), -1);
    }

    #[test]
    fn first_or_default_can_be_any_type() {
        let r =
            builtin_array_first_or(&[ints(&[]), Value::String("fallback".to_string())]).unwrap();
        assert_eq!(as_string(r), "fallback");
    }

    // --- array_last_or ---

    #[test]
    fn last_or_returns_last_when_present() {
        let r = builtin_array_last_or(&[ints(&[10, 20, 30]), Value::Int(-1)]).unwrap();
        assert_eq!(as_int(r), 30);
    }

    #[test]
    fn last_or_returns_default_when_empty() {
        let r = builtin_array_last_or(&[ints(&[]), Value::Int(-1)]).unwrap();
        assert_eq!(as_int(r), -1);
    }

    #[test]
    fn last_or_singleton_is_first() {
        let r = builtin_array_last_or(&[ints(&[42]), Value::Int(-1)]).unwrap();
        assert_eq!(as_int(r), 42);
    }

    // --- general ---

    #[test]
    fn arity_diagnostics_consistent() {
        for f in [
            builtin_array_difference,
            builtin_array_intersection,
            builtin_array_index_of_last,
            builtin_array_first_or,
            builtin_array_last_or,
        ] {
            let err = f(&[]).unwrap_err();
            assert!(err.contains("expected 2"), "got {}", err);
        }
    }

    #[test]
    fn rejects_non_array_first_arg() {
        let err = builtin_array_difference(&[Value::Int(5), ints(&[1])]).unwrap_err();
        assert!(err.contains("expected (array, array)"));
        let err = builtin_array_first_or(&[Value::Int(5), Value::Int(0)]).unwrap_err();
        assert!(err.contains("first argument must be array"));
    }

    #[test]
    fn round_trip_difference_then_union_back_to_original() {
        // For disjoint partition: (a - b) ∪ (a ∩ b) restores a's elements
        // (modulo order, since we're not using a true set).
        let a = ints(&[1, 2, 3, 4, 5]);
        let b = ints(&[2, 4]);
        let diff = builtin_array_difference(&[a.clone(), b.clone()]).unwrap();
        let inter = builtin_array_intersection(&[a, b]).unwrap();
        // |a| = |diff| + |inter|
        match (diff, inter) {
            (Value::Array(d), Value::Array(i)) => {
                assert_eq!(d.len() + i.len(), 5);
            }
            _ => panic!(),
        }
    }
}
