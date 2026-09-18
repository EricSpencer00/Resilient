//! RES-4230: reference-semantics buffers at the FFI boundary.
//!
//! `Buffer<Int>` and `Buffer<Float>` lower to mutable `int64_t*` and
//! `double*` parameters. The borrow guard stays alive for the complete
//! foreign call, so an aliasing buffer cannot be borrowed a second time
//! while C is allowed to mutate its storage.

#![allow(dead_code)]

use crate::ffi::FfiType;
use crate::{BufferStorage, BufferValue};
use std::cell::RefMut;

/// Resolve the supported caller-owned buffer spellings.
pub(crate) fn parse_buffer_type(name: &str) -> Option<Result<FfiType, String>> {
    let inner = name
        .strip_prefix("Buffer<")
        .or_else(|| name.strip_prefix("buffer<"))?
        .strip_suffix('>')?;
    let inner = inner.trim();
    let element = match inner {
        "Int" | "int" => FfiType::Int,
        "Float" | "float" => FfiType::Float,
        _ => return Some(Err(inner.to_string())),
    };
    Some(Ok(FfiType::BufferPtr(Box::new(element))))
}

/// A mutable borrow held across one foreign call.
pub(crate) struct BufferPointer<'a> {
    storage: RefMut<'a, BufferStorage>,
}

impl BufferPointer<'_> {
    /// Return the address of the typed contiguous storage.
    pub(crate) fn ptr(&mut self) -> *mut core::ffi::c_void {
        match &mut *self.storage {
            BufferStorage::Int(items) => items.as_mut_ptr().cast(),
            BufferStorage::Float(items) => items.as_mut_ptr().cast(),
        }
    }
}

/// Borrow a buffer for a mutable C pointer, checking its element type first.
pub(crate) fn borrow_buffer<'a>(
    buffer: &'a BufferValue,
    element: &FfiType,
    fn_name: &str,
    arg_index: usize,
) -> Result<BufferPointer<'a>, String> {
    let storage = buffer.0.try_borrow_mut().map_err(|_| {
        format!(
            "FFI: `{}` arg #{} buffer is already borrowed; mutable BufferPtr access would alias it",
            fn_name, arg_index
        )
    })?;
    let matches = matches!(
        (&*storage, element),
        (BufferStorage::Int(_), FfiType::Int) | (BufferStorage::Float(_), FfiType::Float)
    );
    if !matches {
        let actual = match &*storage {
            BufferStorage::Int(_) => "Buffer<Int>",
            BufferStorage::Float(_) => "Buffer<Float>",
        };
        let expected = match element {
            FfiType::Int => "Buffer<Int>",
            FfiType::Float => "Buffer<Float>",
            other => {
                return Err(format!(
                    "FFI internal: unsupported BufferPtr element {:?}",
                    other
                ));
            }
        };
        return Err(format!(
            "FFI: `{}` arg #{} is declared as {} but received {}",
            fn_name, arg_index, expected, actual
        ));
    }
    Ok(BufferPointer { storage })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_buffer_spellings() {
        assert_eq!(
            parse_buffer_type("Buffer<Int>"),
            Some(Ok(FfiType::BufferPtr(Box::new(FfiType::Int))))
        );
        assert_eq!(
            parse_buffer_type("buffer<float>"),
            Some(Ok(FfiType::BufferPtr(Box::new(FfiType::Float))))
        );
    }

    #[test]
    fn rejects_unsupported_buffer_elements() {
        assert_eq!(
            parse_buffer_type("Buffer<String>"),
            Some(Err("String".to_string()))
        );
        assert_eq!(parse_buffer_type("Array<Int>"), None);
    }

    #[test]
    fn holds_mutable_borrow_until_pointer_guard_is_dropped() {
        let buffer = match crate::buffer_builtins::builtin_buffer_int(&[crate::Value::Int(2)])
            .expect("buffer allocation")
        {
            crate::Value::Buffer(buffer) => buffer,
            other => panic!("expected Buffer, got {other:?}"),
        };
        let mut pointer =
            borrow_buffer(&buffer, &FfiType::Int, "fill", 0).expect("matching buffer type");
        assert!(!pointer.ptr().is_null());
        let error = match borrow_buffer(&buffer, &FfiType::Int, "fill", 0) {
            Ok(_) => panic!("the first mutable borrow must remain active"),
            Err(error) => error,
        };
        assert!(error.contains("already borrowed"), "{error}");
        drop(pointer);
        borrow_buffer(&buffer, &FfiType::Int, "fill", 0)
            .expect("borrow should be available after the call guard drops");
    }
}
