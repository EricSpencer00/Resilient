//! RES-4225: FFI array parameter marshalling (Phase 2a).
//!
//! Lets an `extern fn` take `Array<Int>` / `Array<Float>` and receive it
//! on the C side as `const int64_t*` / `const double*`.
//!
//! `Value::Array` is a `Vec<Value>` — a vector of tagged enums, not a
//! contiguous run of machine words. There is no way to hand C a pointer
//! into it. Marshalling therefore **copies** into a freshly allocated
//! `Vec<i64>` / `Vec<f64>` whose lifetime the caller pins across the
//! foreign call. Callers passing large arrays in a hot loop are paying
//! an O(n) copy per call; that cost is inherent to the value
//! representation, not to this module.
//!
//! Length is *not* passed implicitly. C's convention is a separate
//! count parameter, and inventing a hidden one would make the declared
//! signature disagree with the header. Bindings declare the count
//! themselves:
//!
//! ```text
//! extern "libfoo.so" {
//!     fn sum(xs: Array<Int>, n: Int) -> Int;
//! }
//! ```

// Signature resolution is always compiled; the marshaller is only
// reachable from the `ffi` (dynamic-loading) backend. Mirrors the same
// allowance in `ffi.rs`.
#![allow(dead_code)]

use crate::Value;
use crate::ffi::FfiType;

/// Surface spellings accepted for an array element type, mapped to the
/// FFI scalar it lowers to. Both the capitalised (`Array<Int>`) and
/// lowercase (`array<int>`) spellings reach here because
/// `parse_type_annotation` preserves whatever the source wrote.
fn element_ffi_type(name: &str) -> Option<FfiType> {
    match name {
        "Int" | "int" => Some(FfiType::Int),
        "Float" | "float" => Some(FfiType::Float),
        _ => None,
    }
}

/// Resolve `Array<T>` / `array<T>` to `FfiType::ArrayPtr`.
///
/// Returns `None` for anything that is not an array spelling, so
/// `FfiType::from_resilient` can fall through to its other arms.
/// Returns `Some(Err(elem))` when the shape *is* an array but the
/// element type is not one we can lay out contiguously — that
/// distinction lets the caller emit "Array<String> is unsupported"
/// instead of the much vaguer "unknown type".
pub fn parse_array_type(name: &str) -> Option<Result<FfiType, String>> {
    let inner = name
        .strip_prefix("Array<")
        .or_else(|| name.strip_prefix("array<"))?
        .strip_suffix('>')?;
    let inner = inner.trim();
    match element_ffi_type(inner) {
        Some(elem) => Some(Ok(FfiType::ArrayPtr(Box::new(elem)))),
        None => Some(Err(inner.to_string())),
    }
}

/// A marshalled array buffer, kept alive by the caller for the duration
/// of the foreign call.
///
/// The `Vec` must not be moved or reallocated while C holds the
/// pointer — hence `as_ptr()` is only ever read through `ptr()`, and
/// the buffer is dropped after the call returns.
#[derive(Debug)]
pub enum ArrayBuffer {
    Ints(Vec<i64>),
    Floats(Vec<f64>),
}

impl ArrayBuffer {
    /// Address handed to C. Empty arrays yield a dangling-but-aligned
    /// pointer (`Vec::as_ptr` on an unallocated vec), never null: a
    /// C callee that correctly pairs the pointer with a zero count
    /// will not dereference it, and passing null instead would break
    /// callees that assert non-null before checking the count.
    pub fn ptr(&self) -> *mut core::ffi::c_void {
        match self {
            ArrayBuffer::Ints(v) => v.as_ptr() as *mut core::ffi::c_void,
            ArrayBuffer::Floats(v) => v.as_ptr() as *mut core::ffi::c_void,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            ArrayBuffer::Ints(v) => v.len(),
            ArrayBuffer::Floats(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Copy a `Value::Array` into a contiguous C-compatible buffer.
///
/// `elem` is the element type from the declared signature; every item
/// must match it exactly. Int-to-Float widening is deliberately *not*
/// performed: a silent coercion here would reinterpret the caller's
/// data under a different C type than the one they wrote in the
/// binding, which is precisely the class of mistake FFI declarations
/// exist to prevent.
pub fn marshal_array(
    value: &Value,
    elem: &FfiType,
    fn_name: &str,
    arg_index: usize,
) -> Result<ArrayBuffer, String> {
    let items = match value {
        Value::Array(items) => items,
        other => {
            return Err(format!(
                "FFI: `{}` arg #{}: expected an array, got {:?}",
                fn_name, arg_index, other
            ));
        }
    };

    match elem {
        FfiType::Int => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match item {
                    Value::Int(v) => out.push(*v),
                    other => {
                        return Err(element_mismatch(fn_name, arg_index, i, "Int", other));
                    }
                }
            }
            Ok(ArrayBuffer::Ints(out))
        }
        FfiType::Float => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match item {
                    Value::Float(v) => out.push(*v),
                    other => {
                        return Err(element_mismatch(fn_name, arg_index, i, "Float", other));
                    }
                }
            }
            Ok(ArrayBuffer::Floats(out))
        }
        other => Err(format!(
            "FFI internal: `{}` arg #{} has unsupported array element type {:?}",
            fn_name, arg_index, other
        )),
    }
}

fn element_mismatch(
    fn_name: &str,
    arg_index: usize,
    elem_index: usize,
    want: &str,
    got: &Value,
) -> String {
    format!(
        "FFI: `{}` arg #{} is declared Array<{}> but element [{}] is {:?}; \
         array elements must all match the declared element type",
        fn_name, arg_index, want, elem_index, got
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_capitalised_and_lowercase_spellings() {
        assert_eq!(
            parse_array_type("Array<Int>"),
            Some(Ok(FfiType::ArrayPtr(Box::new(FfiType::Int))))
        );
        assert_eq!(
            parse_array_type("array<int>"),
            Some(Ok(FfiType::ArrayPtr(Box::new(FfiType::Int))))
        );
        assert_eq!(
            parse_array_type("Array<Float>"),
            Some(Ok(FfiType::ArrayPtr(Box::new(FfiType::Float))))
        );
    }

    #[test]
    fn tolerates_interior_whitespace_from_the_encoder() {
        assert_eq!(
            parse_array_type("Array< Int >"),
            Some(Ok(FfiType::ArrayPtr(Box::new(FfiType::Int))))
        );
    }

    #[test]
    fn non_array_names_fall_through() {
        assert_eq!(parse_array_type("Int"), None);
        assert_eq!(parse_array_type("OpaquePtr"), None);
        // A generic that isn't an array must not be claimed by this module.
        assert_eq!(parse_array_type("Option<Int>"), None);
    }

    #[test]
    fn array_of_unsupported_element_reports_the_element_not_the_whole_type() {
        assert_eq!(
            parse_array_type("Array<String>"),
            Some(Err("String".to_string()))
        );
        assert_eq!(
            parse_array_type("Array<Array<Int>>"),
            Some(Err("Array<Int>".to_string()))
        );
    }

    #[test]
    fn marshals_int_array_contiguously() {
        let v = Value::Array(vec![Value::Int(1), Value::Int(-2), Value::Int(3)]);
        let buf = marshal_array(&v, &FfiType::Int, "f", 0).expect("must marshal");
        match &buf {
            ArrayBuffer::Ints(xs) => assert_eq!(xs, &[1, -2, 3]),
            _ => panic!("expected Ints"),
        }
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn marshals_float_array_contiguously() {
        let v = Value::Array(vec![Value::Float(1.5), Value::Float(2.5)]);
        let buf = marshal_array(&v, &FfiType::Float, "f", 0).expect("must marshal");
        match &buf {
            ArrayBuffer::Floats(xs) => assert_eq!(xs, &[1.5, 2.5]),
            _ => panic!("expected Floats"),
        }
    }

    #[test]
    fn empty_array_yields_non_null_pointer_and_zero_len() {
        let v = Value::Array(vec![]);
        let buf = marshal_array(&v, &FfiType::Int, "f", 0).expect("must marshal");
        assert!(buf.is_empty());
        assert!(
            !buf.ptr().is_null(),
            "empty buffers must not marshal to NULL"
        );
    }

    #[test]
    fn mixed_element_types_are_rejected_with_the_offending_index() {
        let v = Value::Array(vec![Value::Int(1), Value::Float(2.0)]);
        let err = marshal_array(&v, &FfiType::Int, "sum", 0).expect_err("must reject");
        assert!(err.contains("[1]"), "err = {}", err);
        assert!(err.contains("Array<Int>"), "err = {}", err);
        assert!(err.contains("sum"), "err = {}", err);
    }

    #[test]
    fn ints_are_not_silently_widened_into_a_float_array() {
        // Declaring Array<Float> and passing Int elements would hand C a
        // bit pattern reinterpreted as a double. Refuse instead.
        let v = Value::Array(vec![Value::Int(1)]);
        let err = marshal_array(&v, &FfiType::Float, "f", 0).expect_err("must reject");
        assert!(err.contains("Array<Float>"), "err = {}", err);
    }

    #[test]
    fn non_array_value_is_rejected() {
        let err = marshal_array(&Value::Int(3), &FfiType::Int, "f", 1).expect_err("must reject");
        assert!(err.contains("expected an array"), "err = {}", err);
    }
}
