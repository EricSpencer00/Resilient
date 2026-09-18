//! RES-4230: reference-semantics output buffers.
//!
//! Buffers are deliberately separate from ordinary arrays. Arrays are
//! value-semantic `Vec<Value>` values, while a `Buffer` is an explicitly
//! allocated, typed region whose clones share storage. The shared storage is
//! the foundation for the later FFI `BufferPtr` lowering; this increment keeps
//! the API useful and testable without requiring a C library.

use crate::{BufferStorage, BufferValue, RResult, Value};

const MAX_BUFFER_ELEMENTS: i64 = 1_000_000_000;

fn checked_len(name: &str, n: i64) -> Result<usize, String> {
    if n < 0 {
        return Err(format!("{name}: length must be non-negative, got {n}"));
    }
    if n > MAX_BUFFER_ELEMENTS {
        return Err(format!(
            "{name}: length {n} too large (max {MAX_BUFFER_ELEMENTS})"
        ));
    }
    Ok(n as usize)
}

fn buffer_arg<'a>(name: &str, args: &'a [Value]) -> Result<&'a BufferValue, String> {
    match args {
        [Value::Buffer(buffer)] => Ok(buffer),
        [other] => Err(format!("{name}: expected buffer, got {other}")),
        _ => Err(format!("{name}: expected 1 argument, got {}", args.len())),
    }
}

fn index_for(name: &str, index: i64, len: usize) -> Result<usize, String> {
    let Ok(index) = usize::try_from(index) else {
        return Err(format!(
            "{name}: index {index} out of bounds for buffer of length {len}"
        ));
    };
    if index >= len {
        return Err(format!(
            "{name}: index {index} out of bounds for buffer of length {len}"
        ));
    }
    Ok(index)
}

/// `buffer_int(n)` allocates a zero-filled shared integer buffer.
pub(crate) fn builtin_buffer_int(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            let len = checked_len("buffer_int", *n)?;
            Ok(Value::Buffer(BufferValue(std::rc::Rc::new(
                std::cell::RefCell::new(BufferStorage::Int(vec![0; len])),
            ))))
        }
        [other] => Err(format!("buffer_int: expected int length, got {other}")),
        _ => Err(format!(
            "buffer_int: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `buffer_float(n)` allocates a zero-filled shared floating-point buffer.
pub(crate) fn builtin_buffer_float(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            let len = checked_len("buffer_float", *n)?;
            Ok(Value::Buffer(BufferValue(std::rc::Rc::new(
                std::cell::RefCell::new(BufferStorage::Float(vec![0.0; len])),
            ))))
        }
        [other] => Err(format!("buffer_float: expected int length, got {other}")),
        _ => Err(format!(
            "buffer_float: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `buffer_len(buffer)` returns the number of elements in the allocation.
pub(crate) fn builtin_buffer_len(args: &[Value]) -> RResult<Value> {
    let buffer = buffer_arg("buffer_len", args)?;
    let len = match &*buffer.0.borrow() {
        BufferStorage::Int(items) => items.len(),
        BufferStorage::Float(items) => items.len(),
    };
    Ok(Value::Int(len as i64))
}

/// `buffer_get(buffer, index)` reads a typed scalar from the shared region.
pub(crate) fn builtin_buffer_get(args: &[Value]) -> RResult<Value> {
    let [Value::Buffer(buffer), Value::Int(index)] = args else {
        return Err(format!(
            "buffer_get: expected (buffer, int), got {} argument(s)",
            args.len()
        ));
    };
    match &*buffer.0.borrow() {
        BufferStorage::Int(items) => Ok(Value::Int(
            items[index_for("buffer_get", *index, items.len())?],
        )),
        BufferStorage::Float(items) => Ok(Value::Float(
            items[index_for("buffer_get", *index, items.len())?],
        )),
    }
}

/// `buffer_set(buffer, index, value)` updates the shared region in place.
pub(crate) fn builtin_buffer_set(args: &[Value]) -> RResult<Value> {
    let [Value::Buffer(buffer), Value::Int(index), value] = args else {
        return Err(format!(
            "buffer_set: expected (buffer, int, scalar), got {} argument(s)",
            args.len()
        ));
    };
    let mut storage = buffer.0.borrow_mut();
    match (&mut *storage, value) {
        (BufferStorage::Int(items), Value::Int(value)) => {
            let slot = index_for("buffer_set", *index, items.len())?;
            items[slot] = *value;
        }
        (BufferStorage::Float(items), Value::Float(value)) => {
            let slot = index_for("buffer_set", *index, items.len())?;
            items[slot] = *value;
        }
        (BufferStorage::Int(_), other) => {
            return Err(format!(
                "buffer_set: integer buffer requires int value, got {other}"
            ));
        }
        (BufferStorage::Float(_), other) => {
            return Err(format!(
                "buffer_set: float buffer requires float value, got {other}"
            ));
        }
    }
    Ok(Value::Void)
}

/// `buffer_to_array(buffer)` snapshots the shared region as an ordinary array.
pub(crate) fn builtin_buffer_to_array(args: &[Value]) -> RResult<Value> {
    let buffer = buffer_arg("buffer_to_array", args)?;
    let values = match &*buffer.0.borrow() {
        BufferStorage::Int(items) => items.iter().copied().map(Value::Int).collect(),
        BufferStorage::Float(items) => items.iter().copied().map(Value::Float).collect(),
    };
    Ok(Value::Array(values))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloned_buffers_share_integer_storage() {
        let original = builtin_buffer_int(&[Value::Int(2)]).unwrap();
        let alias = original.clone();
        builtin_buffer_set(&[alias, Value::Int(1), Value::Int(42)]).unwrap();

        assert!(matches!(
            builtin_buffer_get(&[original, Value::Int(1)]).unwrap(),
            Value::Int(42)
        ));
    }

    #[test]
    fn float_buffers_preserve_type_and_snapshot_values() {
        let buffer = builtin_buffer_float(&[Value::Int(2)]).unwrap();
        builtin_buffer_set(&[buffer.clone(), Value::Int(0), Value::Float(1.25)]).unwrap();

        assert!(matches!(
            builtin_buffer_len(std::slice::from_ref(&buffer)).unwrap(),
            Value::Int(2)
        ));
        assert!(matches!(
            builtin_buffer_get(&[buffer.clone(), Value::Int(0)]).unwrap(),
            Value::Float(value) if (value - 1.25).abs() < f64::EPSILON
        ));
        let Value::Array(values) = builtin_buffer_to_array(&[buffer]).unwrap() else {
            panic!("buffer_to_array must return an array");
        };
        assert!(
            matches!(values.as_slice(), [Value::Float(a), Value::Float(b)]
            if (*a - 1.25).abs() < f64::EPSILON && *b == 0.0)
        );
    }

    #[test]
    fn buffers_reject_invalid_lengths_indices_and_element_types() {
        let negative = builtin_buffer_int(&[Value::Int(-1)]).unwrap_err();
        assert!(negative.contains("non-negative"), "{negative}");

        let buffer = builtin_buffer_int(&[Value::Int(1)]).unwrap();
        let out_of_bounds = builtin_buffer_get(&[buffer.clone(), Value::Int(1)]).unwrap_err();
        assert!(out_of_bounds.contains("out of bounds"), "{out_of_bounds}");

        let wrong_type =
            builtin_buffer_set(&[buffer, Value::Int(0), Value::Float(1.0)]).unwrap_err();
        assert!(wrong_type.contains("requires int"), "{wrong_type}");
    }

    #[test]
    fn interpreter_and_vm_preserve_buffer_identity() {
        let source = r#"
            let samples = buffer_int(2);
            let alias = samples;
            buffer_set(alias, 1, 42);
            buffer_get(samples, 1)
        "#;
        let interpreted = crate::run_program(source);
        assert!(
            interpreted.ok,
            "interpreter errors: {:?}",
            interpreted.errors
        );

        let (program, parse_errors) = crate::parse(source);
        assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
        let bytecode = crate::compiler::compile(&program).expect("source should compile");
        let vm_value = crate::vm::run(&bytecode).expect("VM should run buffer operations");
        assert!(matches!(vm_value, Value::Int(42)));
    }
}
