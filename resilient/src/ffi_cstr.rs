//! RES-4226: `CStr` marshalling and `Int32` width conversion.
//!
//! Two independent gaps in FFI Phase 1, both about the declared type
//! disagreeing with the C ABI:
//!
//! **`CStr`** — `FfiType::Str` marshals to a `(ptr, len)` pair and is
//! only reachable through the `printf`-style variadic path. A plain
//! `fn open(path: CStr) -> OpaquePtr` was unbindable. `CStr` is the
//! ordinary C convention instead: a NUL-terminated `const char*`.
//!
//! **`Int32`** — `FfiType::Int` transmutes to `extern "C" fn(..) -> i64`.
//! A C function returning `int` writes only `eax` / `w0`; the upper 32
//! bits of the return register are not defined by the ABI. Reading them
//! turns a returned `-3` into `4294967293`. `Int32` is the width-correct
//! spelling for C's `int` / `int32_t`.

// Marshalling is only reachable from the `ffi` (dynamic-loading)
// backend; type resolution is always compiled. Mirrors `ffi.rs`.
#![allow(dead_code)]

use crate::Value;
use std::ffi::{CStr as StdCStr, CString};

/// Owned NUL-terminated copy of a Resilient string, kept alive by the
/// caller for the duration of the foreign call.
///
/// Resilient strings are UTF-8 with an explicit length and may contain
/// interior NUL bytes; C strings cannot. The conversion is therefore
/// fallible, and failing loudly is the only sound option — truncating at
/// the first NUL would silently hand C a shorter string than the caller
/// wrote.
#[derive(Debug)]
pub struct CStringBuffer(CString);

impl CStringBuffer {
    pub fn ptr(&self) -> *mut core::ffi::c_void {
        self.0.as_ptr() as *mut core::ffi::c_void
    }
}

/// Copy a `Value::String` into a NUL-terminated buffer for C.
pub fn marshal_cstr(
    value: &Value,
    fn_name: &str,
    arg_index: usize,
) -> Result<CStringBuffer, String> {
    let s = match value {
        Value::String(s) => s,
        other => {
            return Err(format!(
                "FFI: `{}` arg #{}: expected a String for a CStr parameter, got {:?}",
                fn_name, arg_index, other
            ));
        }
    };
    CString::new(s.as_str()).map(CStringBuffer).map_err(|_| {
        format!(
            "FFI: `{}` arg #{} contains an interior NUL byte and cannot be passed as a C string",
            fn_name, arg_index
        )
    })
}

/// Copy a NUL-terminated string returned by C into a Resilient `String`.
///
/// # Ownership
///
/// The pointer is treated as **borrowed and library-owned**. Resilient
/// copies the bytes and never frees them. That matches the common case
/// (`hst_version()`, `strerror()`, any pointer to a static or
/// library-managed buffer) and leaks — but does not corrupt — if the
/// library actually expected the caller to free it.
///
/// This is an assumption the compiler cannot check, in the same family
/// as "the declared signature matches the header". See
/// `docs/ffi-trust-boundary.md`.
///
/// # Safety
///
/// The caller must ensure `ptr` is either null or points to a
/// NUL-terminated byte sequence that stays valid for the duration of
/// this call. That holds when the `extern` declaration naming this
/// return type is accurate — the same precondition every other
/// trampoline arm rests on.
pub unsafe fn cstr_return_to_value(
    ptr: *const core::ffi::c_char,
    fn_name: &str,
) -> Result<Value, String> {
    if ptr.is_null() {
        return Err(format!(
            "FFI: `{}` returned a null C string; declare the return as OpaquePtr if null is a valid result",
            fn_name
        ));
    }
    // SAFETY: non-null checked above; NUL-termination and validity are
    // the caller's documented precondition.
    let bytes = unsafe { StdCStr::from_ptr(ptr) }.to_bytes();
    match std::str::from_utf8(bytes) {
        Ok(s) => Ok(Value::String(s.to_string())),
        Err(_) => Err(format!(
            "FFI: `{}` returned a C string that is not valid UTF-8",
            fn_name
        )),
    }
}

/// Narrow a Resilient `Int` to C's `int` for an `Int32` parameter.
///
/// Refuses rather than truncating. A silent wrap would hand C a
/// different number than the caller wrote — the exact failure this type
/// exists to prevent on the return side.
pub fn narrow_to_i32(v: i64, fn_name: &str, arg_index: usize) -> Result<i32, String> {
    i32::try_from(v).map_err(|_| {
        format!(
            "FFI: `{}` arg #{} is {} which does not fit in Int32 (C `int`, range {}..={})",
            fn_name,
            arg_index,
            v,
            i32::MIN,
            i32::MAX
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marshals_a_plain_string() {
        let v = Value::String("hello".to_string());
        let buf = marshal_cstr(&v, "f", 0).expect("must marshal");
        // SAFETY: buf owns a valid NUL-terminated CString.
        let back = unsafe { StdCStr::from_ptr(buf.ptr() as *const core::ffi::c_char) };
        assert_eq!(back.to_str().unwrap(), "hello");
    }

    #[test]
    fn marshals_an_empty_string_as_a_bare_nul() {
        let v = Value::String(String::new());
        let buf = marshal_cstr(&v, "f", 0).expect("must marshal");
        let back = unsafe { StdCStr::from_ptr(buf.ptr() as *const core::ffi::c_char) };
        assert_eq!(back.to_bytes().len(), 0);
    }

    #[test]
    fn interior_nul_is_rejected_not_truncated() {
        let v = Value::String("a\0b".to_string());
        let err = marshal_cstr(&v, "open", 1).expect_err("must reject");
        assert!(err.contains("interior NUL"), "err = {}", err);
        assert!(err.contains("open"), "err = {}", err);
    }

    #[test]
    fn non_string_argument_is_rejected() {
        let err = marshal_cstr(&Value::Int(1), "f", 0).expect_err("must reject");
        assert!(err.contains("expected a String"), "err = {}", err);
    }

    #[test]
    fn null_return_is_a_clean_error_not_a_panic() {
        let err = unsafe { cstr_return_to_value(std::ptr::null(), "version") }
            .expect_err("null must error");
        assert!(err.contains("null C string"), "err = {}", err);
    }

    #[test]
    fn round_trips_a_returned_c_string() {
        let owned = CString::new("hstcore 1.4.0").unwrap();
        let v = unsafe { cstr_return_to_value(owned.as_ptr(), "version") }.expect("must convert");
        match v {
            Value::String(s) => assert_eq!(s, "hstcore 1.4.0"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn invalid_utf8_return_is_rejected() {
        let owned = CString::new(vec![0xffu8, 0xfe]).unwrap();
        let err = unsafe { cstr_return_to_value(owned.as_ptr(), "f") }.expect_err("must reject");
        assert!(err.contains("not valid UTF-8"), "err = {}", err);
    }

    #[test]
    fn narrowing_accepts_the_full_i32_range() {
        assert_eq!(narrow_to_i32(0, "f", 0).unwrap(), 0);
        assert_eq!(narrow_to_i32(-3, "f", 0).unwrap(), -3);
        assert_eq!(narrow_to_i32(i32::MAX as i64, "f", 0).unwrap(), i32::MAX);
        assert_eq!(narrow_to_i32(i32::MIN as i64, "f", 0).unwrap(), i32::MIN);
    }

    #[test]
    fn narrowing_refuses_to_wrap() {
        let err = narrow_to_i32(i32::MAX as i64 + 1, "set_count", 2).expect_err("must reject");
        assert!(err.contains("does not fit in Int32"), "err = {}", err);
        assert!(err.contains("set_count"), "err = {}", err);
    }
}
