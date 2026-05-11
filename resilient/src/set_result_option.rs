//! RES-1154: small gap-filling builtins for Set / Result / Option.
//!
//! - `set_is_empty`: parallel to RES-1144's `map_is_empty`.
//! - `set_from_array`: build a Set from an Array of hashable values.
//! - `result_and` / `option_and`: short-circuit combinators that
//!   complete the `_or` / `_and` pair (the `_or` variants already
//!   ship — see RES-939).
//!
//! All four are pure leaf builtins.

use crate::{MapKey, RResult, Value};
use std::collections::HashSet;

/// `set_is_empty(s) -> Bool` — true iff the set has zero items.
pub(crate) fn builtin_set_is_empty(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Set(s)] => Ok(Value::Bool(s.is_empty())),
        [a] => Err(format!("set_is_empty: expected a Set, got {}", a)),
        _ => Err(format!(
            "set_is_empty: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `set_from_array(arr) -> Set` — build a Set from an Array. Each
/// element must be Int / String / Bool (the hashable subset that
/// `MapKey` accepts). Duplicates collapse to one entry.
pub(crate) fn builtin_set_from_array(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            let mut s = HashSet::new();
            for v in items {
                let k = MapKey::from_value(v)?;
                s.insert(k);
            }
            Ok(Value::Set(s))
        }
        [a] => Err(format!("set_from_array: expected an Array, got {}", a)),
        _ => Err(format!(
            "set_from_array: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `result_and(a, b) -> Result` — return `a` if it is `Err`, otherwise
/// return `b`. Matches Rust's `Result::and` short-circuit semantics:
/// once the first error appears, propagate it instead of running the
/// rest of the chain.
pub(crate) fn builtin_result_and(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Result { ok: false, .. }, _] => Ok(args[0].clone()),
        [Value::Result { ok: true, .. }, Value::Result { .. }] => Ok(args[1].clone()),
        [Value::Result { .. }, other] => Err(format!(
            "result_and: second argument must be Result, got {}",
            other
        )),
        [other, _] => Err(format!(
            "result_and: first argument must be Result, got {}",
            other
        )),
        _ => Err(format!(
            "result_and: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `option_and(a, b) -> Option` — return `None` if `a` is `None`,
/// otherwise return `b`. Matches Rust's `Option::and`.
pub(crate) fn builtin_option_and(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Option(None), _] => Ok(Value::Option(None)),
        [Value::Option(Some(_)), Value::Option(_)] => Ok(args[1].clone()),
        [Value::Option(_), other] => Err(format!(
            "option_and: second argument must be Option, got {}",
            other
        )),
        [other, _] => Err(format!(
            "option_and: first argument must be Option, got {}",
            other
        )),
        _ => Err(format!(
            "option_and: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_with(keys: &[MapKey]) -> Value {
        let mut s = HashSet::new();
        for k in keys {
            s.insert(k.clone());
        }
        Value::Set(s)
    }

    fn ok(v: Value) -> Value {
        Value::Result {
            ok: true,
            payload: Box::new(v),
        }
    }

    fn err(v: Value) -> Value {
        Value::Result {
            ok: false,
            payload: Box::new(v),
        }
    }

    fn some(v: Value) -> Value {
        Value::Option(Some(Box::new(v)))
    }

    fn none() -> Value {
        Value::Option(None)
    }

    // --- set_is_empty ---

    #[test]
    fn set_is_empty_basic() {
        assert!(matches!(
            builtin_set_is_empty(&[set_with(&[])]).unwrap(),
            Value::Bool(true)
        ));
        assert!(matches!(
            builtin_set_is_empty(&[set_with(&[MapKey::Int(1)])]).unwrap(),
            Value::Bool(false)
        ));
    }

    #[test]
    fn set_is_empty_rejects_non_set() {
        let err = builtin_set_is_empty(&[Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected a Set"));
    }

    #[test]
    fn set_is_empty_rejects_wrong_arity() {
        let err = builtin_set_is_empty(&[]).unwrap_err();
        assert!(err.contains("expected 1"));
    }

    // --- set_from_array ---

    #[test]
    fn set_from_array_basic() {
        let arr = Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        let s = builtin_set_from_array(&[arr]).unwrap();
        match s {
            Value::Set(set) => {
                assert_eq!(set.len(), 3);
                assert!(set.contains(&MapKey::Int(1)));
                assert!(set.contains(&MapKey::Int(2)));
                assert!(set.contains(&MapKey::Int(3)));
            }
            _ => panic!("expected Set"),
        }
    }

    #[test]
    fn set_from_array_deduplicates() {
        let arr = Value::Array(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(1),
            Value::Int(2),
        ]);
        let s = builtin_set_from_array(&[arr]).unwrap();
        match s {
            Value::Set(set) => assert_eq!(set.len(), 2),
            _ => panic!("expected Set"),
        }
    }

    #[test]
    fn set_from_array_empty() {
        let s = builtin_set_from_array(&[Value::Array(vec![])]).unwrap();
        match s {
            Value::Set(set) => assert_eq!(set.len(), 0),
            _ => panic!("expected Set"),
        }
    }

    #[test]
    fn set_from_array_mixed_hashable_types() {
        let arr = Value::Array(vec![
            Value::Int(1),
            Value::String("hello".to_string()),
            Value::Bool(true),
        ]);
        let s = builtin_set_from_array(&[arr]).unwrap();
        match s {
            Value::Set(set) => {
                assert_eq!(set.len(), 3);
                assert!(set.contains(&MapKey::Int(1)));
                assert!(set.contains(&MapKey::Str("hello".to_string())));
                assert!(set.contains(&MapKey::Bool(true)));
            }
            _ => panic!("expected Set"),
        }
    }

    #[test]
    fn set_from_array_rejects_non_hashable_element() {
        let arr = Value::Array(vec![Value::Int(1), Value::Float(2.0)]);
        let err = builtin_set_from_array(&[arr]).unwrap_err();
        assert!(err.contains("Map key must be Int, String, or Bool"));
    }

    #[test]
    fn set_from_array_rejects_non_array() {
        let err = builtin_set_from_array(&[Value::Int(5)]).unwrap_err();
        assert!(err.contains("expected an Array"));
    }

    // --- result_and ---

    #[test]
    fn result_and_short_circuits_on_first_err() {
        let r = builtin_result_and(&[err(Value::String("first".to_string())), ok(Value::Int(2))])
            .unwrap();
        match r {
            Value::Result { ok, payload } => {
                assert!(!ok);
                match *payload {
                    Value::String(s) => assert_eq!(s, "first"),
                    _ => panic!("expected String payload"),
                }
            }
            _ => panic!("expected Result"),
        }
    }

    #[test]
    fn result_and_returns_second_when_first_is_ok() {
        let r = builtin_result_and(&[ok(Value::Int(1)), ok(Value::Int(2))]).unwrap();
        match r {
            Value::Result { ok, payload } => {
                assert!(ok);
                match *payload {
                    Value::Int(n) => assert_eq!(n, 2),
                    _ => panic!("expected Int payload"),
                }
            }
            _ => panic!("expected Result"),
        }
    }

    #[test]
    fn result_and_propagates_second_err() {
        let r = builtin_result_and(&[ok(Value::Int(1)), err(Value::String("second".to_string()))])
            .unwrap();
        match r {
            Value::Result { ok, .. } => assert!(!ok),
            _ => panic!("expected Result"),
        }
    }

    #[test]
    fn result_and_rejects_non_result() {
        let err = builtin_result_and(&[ok(Value::Int(1)), Value::Int(2)]).unwrap_err();
        assert!(err.contains("second argument must be Result"));
        let err = builtin_result_and(&[Value::Int(1), ok(Value::Int(2))]).unwrap_err();
        assert!(err.contains("first argument must be Result"));
    }

    // --- option_and ---

    #[test]
    fn option_and_short_circuits_on_first_none() {
        let r = builtin_option_and(&[none(), some(Value::Int(2))]).unwrap();
        assert!(matches!(r, Value::Option(None)));
    }

    #[test]
    fn option_and_returns_second_when_first_is_some() {
        let r = builtin_option_and(&[some(Value::Int(1)), some(Value::Int(2))]).unwrap();
        match r {
            Value::Option(Some(inner)) => match *inner {
                Value::Int(n) => assert_eq!(n, 2),
                _ => panic!("expected Int payload"),
            },
            _ => panic!("expected Some"),
        }
    }

    #[test]
    fn option_and_propagates_second_none() {
        let r = builtin_option_and(&[some(Value::Int(1)), none()]).unwrap();
        assert!(matches!(r, Value::Option(None)));
    }

    #[test]
    fn option_and_rejects_non_option() {
        let err = builtin_option_and(&[some(Value::Int(1)), Value::Int(2)]).unwrap_err();
        assert!(err.contains("second argument must be Option"));
        let err = builtin_option_and(&[Value::Int(1), some(Value::Int(2))]).unwrap_err();
        assert!(err.contains("first argument must be Option"));
    }

    // --- general ---

    #[test]
    fn arity_diagnostics_consistent() {
        for f in [builtin_result_and, builtin_option_and] {
            let err = f(&[]).unwrap_err();
            assert!(err.contains("expected 2"), "got {}", err);
            let err = f(&[Value::Int(1)]).unwrap_err();
            assert!(err.contains("expected 2"), "got {}", err);
        }
    }
}
