//! `array_sort_by_field` / `array_sort_by_field_desc` — sort an array of
//! structs or maps by a named field without writing a comparator callback.
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `array_sort_by_field(arr, field)` | `(Array, String) -> Array` | ascending |
//! | `array_sort_by_field_desc(arr, field)` | `(Array, String) -> Array` | descending |
//!
//! Field values are compared using the natural order: Int < Float < String.
//! Cross-type comparisons fall back to string representation so they never
//! error — elements whose field is absent sort last.

use crate::{RResult, Value};
use std::cmp::Ordering;

fn field_value<'a>(element: &'a Value, field: &str) -> Option<&'a Value> {
    match element {
        Value::Struct { fields, .. } => fields.iter().find(|(k, _)| k == field).map(|(_, v)| v),
        Value::Map(m) => m.get(&crate::MapKey::Str(field.to_string())),
        _ => None,
    }
}

fn cmp_values(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Int(x), Value::Float(y)) => (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Float(x), Value::Int(y)) => x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => a.to_string().cmp(&b.to_string()),
    }
}

fn sort_impl(args: &[Value], descending: bool, fname: &str) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::String(field)] => {
            let field = field.clone();
            let mut indexed: Vec<(usize, Value)> = items.iter().cloned().enumerate().collect();

            indexed.sort_by(|(_, a), (_, b)| {
                let va = field_value(a, &field);
                let vb = field_value(b, &field);
                let ord = match (va, vb) {
                    (Some(fa), Some(fb)) => cmp_values(fa, fb),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                };
                if descending { ord.reverse() } else { ord }
            });

            Ok(Value::Array(indexed.into_iter().map(|(_, v)| v).collect()))
        }
        [a, b] => Err(format!(
            "{}: expected (array, string), got ({}, {})",
            fname, a, b
        )),
        _ => Err(format!(
            "{}: expected 2 arguments (array, field_name), got {}",
            fname,
            args.len()
        )),
    }
}

/// `array_sort_by_field(arr, field)` — ascending sort of structs/maps by a
/// named field. Elements missing the field sort last.
pub(crate) fn builtin_array_sort_by_field(args: &[Value]) -> RResult<Value> {
    sort_impl(args, false, "array_sort_by_field")
}

/// `array_sort_by_field_desc(arr, field)` — descending sort of structs/maps
/// by a named field. Elements missing the field sort last.
pub(crate) fn builtin_array_sort_by_field_desc(args: &[Value]) -> RResult<Value> {
    sort_impl(args, true, "array_sort_by_field_desc")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_struct(name: &str, age: i64) -> Value {
        Value::Struct {
            name: "Person".to_string(),
            fields: vec![
                ("name".to_string(), Value::String(name.to_string())),
                ("age".to_string(), Value::Int(age)),
            ],
        }
    }

    fn get_int_field(v: &Value, fname: &str) -> i64 {
        if let Value::Struct { fields, .. } = v
            && let Some((_, Value::Int(n))) = fields.iter().find(|(k, _)| k == fname)
        {
            return *n;
        }
        panic!("field {} not found or not Int in {:?}", fname, v);
    }

    fn get_str_field(v: &Value, fname: &str) -> String {
        if let Value::Struct { fields, .. } = v
            && let Some((_, Value::String(s))) = fields.iter().find(|(k, _)| k == fname)
        {
            return s.clone();
        }
        panic!("field {} not found or not String in {:?}", fname, v);
    }

    #[test]
    fn sort_structs_by_int_field_ascending() {
        let arr = Value::Array(vec![
            make_struct("Charlie", 30),
            make_struct("Alice", 20),
            make_struct("Bob", 25),
        ]);
        let result = builtin_array_sort_by_field(&[arr, Value::String("age".to_string())]).unwrap();
        if let Value::Array(items) = result {
            assert_eq!(items.len(), 3);
            assert_eq!(get_int_field(&items[0], "age"), 20);
            assert_eq!(get_int_field(&items[1], "age"), 25);
            assert_eq!(get_int_field(&items[2], "age"), 30);
        } else {
            panic!("expected Array");
        }
    }

    #[test]
    fn sort_structs_by_string_field_ascending() {
        let arr = Value::Array(vec![
            make_struct("Charlie", 30),
            make_struct("Alice", 20),
            make_struct("Bob", 25),
        ]);
        let result =
            builtin_array_sort_by_field(&[arr, Value::String("name".to_string())]).unwrap();
        if let Value::Array(items) = result {
            assert_eq!(get_str_field(&items[0], "name"), "Alice");
            assert_eq!(get_str_field(&items[1], "name"), "Bob");
            assert_eq!(get_str_field(&items[2], "name"), "Charlie");
        } else {
            panic!("expected Array");
        }
    }

    #[test]
    fn sort_structs_descending() {
        let arr = Value::Array(vec![make_struct("Alice", 20), make_struct("Bob", 25)]);
        let result =
            builtin_array_sort_by_field_desc(&[arr, Value::String("age".to_string())]).unwrap();
        if let Value::Array(items) = result {
            assert_eq!(get_int_field(&items[0], "age"), 25);
            assert_eq!(get_int_field(&items[1], "age"), 20);
        } else {
            panic!("expected Array");
        }
    }

    #[test]
    fn missing_field_sorts_last() {
        let has_field = make_struct("Alice", 20);
        let no_field = Value::Struct {
            name: "Person".to_string(),
            fields: vec![("name".to_string(), Value::String("Zara".to_string()))],
        };
        let arr = Value::Array(vec![no_field, has_field]);
        let result = builtin_array_sort_by_field(&[arr, Value::String("age".to_string())]).unwrap();
        if let Value::Array(items) = result {
            if let Value::Struct { fields, .. } = &items[1] {
                assert!(!fields.iter().any(|(k, _)| k == "age"));
            } else {
                panic!("expected Struct at index 1");
            }
        } else {
            panic!("expected Array");
        }
    }

    #[test]
    fn rejects_wrong_types() {
        let err = builtin_array_sort_by_field(&[Value::Int(1), Value::String("x".to_string())])
            .unwrap_err();
        assert!(err.contains("expected (array, string)"));
    }

    #[test]
    fn rejects_wrong_arity() {
        let err = builtin_array_sort_by_field(&[Value::Array(vec![])]).unwrap_err();
        assert!(err.contains("expected 2 arguments"));
    }

    #[test]
    fn sort_maps_by_field() {
        use crate::MapKey;
        let make_map = |age: i64| {
            let mut m = HashMap::new();
            m.insert(MapKey::Str("age".to_string()), Value::Int(age));
            Value::Map(m)
        };
        let arr = Value::Array(vec![make_map(30), make_map(10), make_map(20)]);
        let result = builtin_array_sort_by_field(&[arr, Value::String("age".to_string())]).unwrap();
        if let Value::Array(items) = result {
            if let Value::Map(m) = &items[0] {
                match m.get(&MapKey::Str("age".to_string())) {
                    Some(Value::Int(10)) => {}
                    other => panic!("expected Int(10) for 'age', got {:?}", other),
                }
            } else {
                panic!("expected Map");
            }
        } else {
            panic!("expected Array");
        }
    }
}
