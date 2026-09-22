use super::builtin_array_flatten_depth;
use crate::Value;

fn array(values: Vec<Value>) -> Value {
    Value::Array(values)
}

fn assert_value_shape(actual: &Value, expected: &Value) {
    match (actual, expected) {
        (Value::Int(actual), Value::Int(expected)) => assert_eq!(actual, expected),
        (Value::Array(actual), Value::Array(expected)) => {
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.iter().zip(expected) {
                assert_value_shape(actual, expected);
            }
        }
        _ => panic!("value shapes differ: actual={actual:?}, expected={expected:?}"),
    }
}

#[test]
fn deeply_nested_input_does_not_exhaust_native_stack() {
    let mut deep = Value::Int(42);
    for _ in 0..20_000 {
        deep = array(vec![deep]);
    }

    let args = [deep, Value::Int(i64::MAX)];
    let result = builtin_array_flatten_depth(&args).unwrap();
    let Value::Array(values) = result else {
        panic!("expected flattened array");
    };
    assert!(matches!(values.as_slice(), [Value::Int(42)]));
    std::mem::forget(args);
}

#[test]
fn worklist_preserves_order_and_depth_boundaries() {
    let input = array(vec![
        array(vec![Value::Int(1), array(vec![Value::Int(2)])]),
        Value::Int(3),
    ]);

    let depth_one = builtin_array_flatten_depth(&[input.clone(), Value::Int(1)]).unwrap();
    assert_value_shape(
        &depth_one,
        &array(vec![
            Value::Int(1),
            array(vec![Value::Int(2)]),
            Value::Int(3),
        ]),
    );

    let depth_two = builtin_array_flatten_depth(&[input.clone(), Value::Int(2)]).unwrap();
    assert_value_shape(
        &depth_two,
        &array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );

    let fully_flattened = builtin_array_flatten_depth(&[input, Value::Int(3)]).unwrap();
    assert_value_shape(
        &fully_flattened,
        &array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
}
