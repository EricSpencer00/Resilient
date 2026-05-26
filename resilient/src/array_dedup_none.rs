//! `array_dedup_by` and `array_none` — two missing higher-order array operations.
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `array_dedup_by(arr, field)` | `(Array, String) -> Array` | remove duplicate structs/maps by field |
//! | `array_none(arr, pred)` | `(Array, Fn) -> Bool` | true iff no element satisfies pred |
//!
//! `array_dedup_by` keeps the **first** element with each field value, preserving
//! insertion order. `array_none` is the logical complement of `array_any`.

use crate::{Interpreter, RResult, Value};
use std::collections::HashSet;

/// `array_dedup_by(arr, field)` — deduplicate an array of structs or maps,
/// keeping the first element for each distinct value of `field`.
pub(crate) fn builtin_array_dedup_by(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::String(field)] => {
            let mut seen: HashSet<String> = HashSet::new();
            let mut out = Vec::new();
            for item in items {
                let key_val = match item {
                    Value::Struct { fields, .. } => fields
                        .iter()
                        .find(|(k, _)| k == field)
                        .map(|(_, v)| v.to_string()),
                    Value::Map(m) => m
                        .get(&crate::MapKey::Str(field.to_string()))
                        .map(|v| v.to_string()),
                    _ => None,
                };
                let key = key_val.unwrap_or_else(|| "__absent__".to_string());
                if seen.insert(key) {
                    out.push(item.clone());
                }
            }
            Ok(Value::Array(out))
        }
        [a, b] => Err(format!(
            "array_dedup_by: expected (array, string), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "array_dedup_by: expected 2 arguments (array, field), got {}",
            args.len()
        )),
    }
}

/// `array_none(arr, pred)` — returns `true` iff no element of `arr` satisfies
/// `pred`. The logical complement of `array_any`.
pub(crate) fn builtin_array_none(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), pred] => {
            for item in items {
                match interp.apply_function(pred, vec![item.clone()])? {
                    Value::Bool(true) => return Ok(Value::Bool(false)),
                    Value::Bool(false) => {}
                    other => {
                        return Err(format!(
                            "array_none: predicate must return Bool, got {}",
                            other
                        ));
                    }
                }
            }
            Ok(Value::Bool(true))
        }
        [a, _] => Err(format!(
            "array_none: first argument must be an Array, got {}",
            a
        )),
        _ => Err(format!(
            "array_none: expected 2 arguments (array, predicate), got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_struct(id: i64, name: &str) -> Value {
        Value::Struct {
            name: "Item".to_string(),
            fields: vec![
                ("id".to_string(), Value::Int(id)),
                ("name".to_string(), Value::String(name.to_string())),
            ],
        }
    }

    #[test]
    fn dedup_keeps_first_occurrence() {
        let arr = Value::Array(vec![
            make_struct(1, "alpha"),
            make_struct(2, "beta"),
            make_struct(1, "alpha-dup"),
        ]);
        let result = builtin_array_dedup_by(&[arr, Value::String("id".to_string())]).unwrap();
        if let Value::Array(items) = result {
            assert_eq!(items.len(), 2);
            if let Value::Struct { fields, .. } = &items[0] {
                match fields.iter().find(|(k, _)| k == "name") {
                    Some((_, Value::String(s))) => assert_eq!(s, "alpha"),
                    other => panic!("expected name=alpha, got {:?}", other),
                }
            } else {
                panic!("expected Struct");
            }
        } else {
            panic!("expected Array");
        }
    }

    #[test]
    fn dedup_empty_array() {
        let result =
            builtin_array_dedup_by(&[Value::Array(vec![]), Value::String("id".to_string())])
                .unwrap();
        match result {
            Value::Array(items) => assert!(items.is_empty()),
            other => panic!("expected empty Array, got {:?}", other),
        }
    }

    #[test]
    fn dedup_no_duplicates() {
        let arr = Value::Array(vec![make_struct(1, "a"), make_struct(2, "b")]);
        let result = builtin_array_dedup_by(&[arr, Value::String("id".to_string())]).unwrap();
        if let Value::Array(items) = result {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected Array");
        }
    }

    #[test]
    fn dedup_rejects_wrong_types() {
        let err =
            builtin_array_dedup_by(&[Value::Int(1), Value::String("id".to_string())]).unwrap_err();
        assert!(err.contains("expected (array, string)"));
    }

    #[test]
    fn dedup_rejects_wrong_arity() {
        let err = builtin_array_dedup_by(&[Value::Array(vec![])]).unwrap_err();
        assert!(err.contains("expected 2 arguments"));
    }
}
