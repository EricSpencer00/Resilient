use super::*;

fn row(score: Option<Value>) -> Value {
    let mut fields = vec![("name".to_string(), Value::String("row".to_string()))];
    if let Some(score) = score {
        fields.push(("score".to_string(), score));
    }
    Value::Struct {
        name: "Row".to_string(),
        fields,
    }
}

#[test]
fn descending_sort_keeps_missing_fields_last() {
    let result = builtin_array_sort_by_field_desc(&[
        Value::Array(vec![row(None), row(Some(Value::Int(10)))]),
        Value::String("score".to_string()),
    ])
    .expect("sort should succeed");

    let Value::Array(items) = result else {
        panic!("expected sorted array");
    };
    assert!(field_value(&items[0], "score").is_some());
    assert!(field_value(&items[1], "score").is_none());
}

#[test]
fn float_sort_uses_total_order_for_nan() {
    let result = builtin_array_sort_by_field(&[
        Value::Array(vec![
            row(Some(Value::Float(f64::NAN))),
            row(Some(Value::Float(1.0))),
            row(Some(Value::Int(0))),
        ]),
        Value::String("score".to_string()),
    ])
    .expect("sort should succeed");

    let Value::Array(items) = result else {
        panic!("expected sorted array");
    };
    assert!(matches!(
        field_value(&items[0], "score"),
        Some(Value::Int(0))
    ));
    assert!(matches!(field_value(&items[1], "score"), Some(Value::Float(value)) if *value == 1.0));
    assert!(matches!(field_value(&items[2], "score"), Some(Value::Float(value)) if value.is_nan()));
}
