use super::builtin_array_flatten_depth;
use crate::Value;

fn array(values: Vec<Value>) -> Value {
    Value::Array(values)
}

#[test]
fn deeply_nested_input_does_not_exhaust_native_stack() {
    let mut deep = Value::Int(42);
    for _ in 0..20_000 {
        deep = array(vec![deep]);
    }

    let result = builtin_array_flatten_depth(&[deep, Value::Int(i64::MAX)]).unwrap();
    let Value::Array(values) = result else {
        panic!("expected flattened array");
    };
    assert_eq!(values, vec![Value::Int(42)]);
}

#[test]
fn worklist_preserves_order_and_depth_boundaries() {
    let input = array(vec![
        array(vec![Value::Int(1), array(vec![Value::Int(2)])]),
        Value::Int(3),
    ]);

    let depth_one = builtin_array_flatten_depth(&[input.clone(), Value::Int(1)]).unwrap();
    assert_eq!(
        depth_one,
        array(vec![
            array(vec![Value::Int(1), array(vec![Value::Int(2)])]),
            Value::Int(3)
        ])
    );

    let depth_two = builtin_array_flatten_depth(&[input.clone(), Value::Int(2)]).unwrap();
    assert_eq!(
        depth_two,
        array(vec![
            Value::Int(1),
            array(vec![Value::Int(2)]),
            Value::Int(3)
        ])
    );

    let fully_flattened = builtin_array_flatten_depth(&[input, Value::Int(3)]).unwrap();
    assert_eq!(
        fully_flattened,
        array(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}
