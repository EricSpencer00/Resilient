//! `array_flatten_depth(arr, depth)` — recursively flatten nested arrays up to
//! `depth` levels deep. `depth = 0` returns the array unchanged; `depth = 1`
//! matches the existing `array_flatten`; a very large depth (e.g. `999`)
//! flattens fully.
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `array_flatten_depth(arr, depth)` | `(Array, Int) -> Array` | depth-bounded flatten |

use crate::{RResult, Value};

fn flatten_rec(items: &[Value], depth: i64, out: &mut Vec<Value>) {
    for v in items {
        if depth > 0
            && let Value::Array(inner) = v
        {
            flatten_rec(inner, depth - 1, out);
            continue;
        }
        out.push(v.clone());
    }
}

/// `array_flatten_depth(arr, depth)` — recursively flatten `arr` at most
/// `depth` levels. `depth = 0` is a no-op; negative depth is an error.
pub(crate) fn builtin_array_flatten_depth(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::Int(depth)] => {
            if *depth < 0 {
                return Err(format!(
                    "array_flatten_depth: depth must be non-negative, got {}",
                    depth
                ));
            }
            let mut out = Vec::new();
            flatten_rec(items, *depth, &mut out);
            Ok(Value::Array(out))
        }
        [a, b] => Err(format!(
            "array_flatten_depth: expected (array, int), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "array_flatten_depth: expected 2 arguments (array, depth), got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arr(items: Vec<Value>) -> Value {
        Value::Array(items)
    }

    fn unwrap_array(v: Value) -> Vec<Value> {
        match v {
            Value::Array(items) => items,
            other => panic!("expected Array, got {:?}", other),
        }
    }

    fn unwrap_int(v: &Value) -> i64 {
        match v {
            Value::Int(n) => *n,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn depth_zero_is_noop() {
        let input = arr(vec![arr(vec![Value::Int(1)]), Value::Int(2)]);
        let result = unwrap_array(builtin_array_flatten_depth(&[input, Value::Int(0)]).unwrap());
        // First element is still an Array (not flattened)
        assert!(matches!(result[0], Value::Array(_)));
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn depth_one_matches_flatten() {
        let input = arr(vec![arr(vec![Value::Int(1), Value::Int(2)]), Value::Int(3)]);
        let result = unwrap_array(builtin_array_flatten_depth(&[input, Value::Int(1)]).unwrap());
        assert_eq!(result.len(), 3);
        assert_eq!(unwrap_int(&result[0]), 1);
        assert_eq!(unwrap_int(&result[1]), 2);
        assert_eq!(unwrap_int(&result[2]), 3);
    }

    #[test]
    fn depth_two_flattens_two_levels() {
        let inner = arr(vec![arr(vec![Value::Int(1), Value::Int(2)])]);
        let input = arr(vec![inner]);
        let result = unwrap_array(builtin_array_flatten_depth(&[input, Value::Int(2)]).unwrap());
        assert_eq!(result.len(), 2);
        assert_eq!(unwrap_int(&result[0]), 1);
        assert_eq!(unwrap_int(&result[1]), 2);
    }

    #[test]
    fn depth_one_leaves_deeper_nesting() {
        let deep = arr(vec![arr(vec![Value::Int(1)])]);
        let input = arr(vec![deep]);
        let result = unwrap_array(builtin_array_flatten_depth(&[input, Value::Int(1)]).unwrap());
        // Depth 1: outer array unwrapped once, inner still a nested Array
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], Value::Array(_)));
    }

    #[test]
    fn large_depth_flattens_fully() {
        let deep = arr(vec![arr(vec![arr(vec![Value::Int(42)])])]);
        let result = unwrap_array(builtin_array_flatten_depth(&[deep, Value::Int(999)]).unwrap());
        assert_eq!(result.len(), 1);
        assert_eq!(unwrap_int(&result[0]), 42);
    }

    #[test]
    fn negative_depth_errors() {
        let err = builtin_array_flatten_depth(&[arr(vec![]), Value::Int(-1)]).unwrap_err();
        assert!(err.contains("non-negative"));
    }

    #[test]
    fn rejects_wrong_types() {
        let err = builtin_array_flatten_depth(&[Value::Int(1), Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected (array, int)"));
    }

    #[test]
    fn rejects_wrong_arity() {
        let err = builtin_array_flatten_depth(&[arr(vec![])]).unwrap_err();
        assert!(err.contains("expected 2 arguments"));
    }
}
