//! FFI v1: trampolines dispatch a resolved `ForeignSymbol` to a real
//! C function pointer of the right type.
//!
//! Input `Value`s are converted to C ABI scalars here. Output C
//! scalars are converted back to `Value`. String marshalling uses
//! `(*const u8, usize)` — the trampoline holds the Resilient String's
//! UTF-8 bytes alive on the stack for the duration of the call via
//! the `live_strs` vector.
//!
//! Coverage: arity 0-2 with the primitive combinations the Phase 1
//! tests need (libm `cos`, `sqrt`, plus a handful of helper fns).
//! Unsupported signatures return a clean error referencing the
//! `(params, ret)` tuple that was missing, so extending the table
//! is mechanical when new call shapes appear.
use crate::ffi::{FfiError, FfiType, ForeignSymbol};
use crate::{RResult, Value};

#[allow(dead_code)]
#[derive(Copy, Clone)]
#[repr(C)]
struct CStr {
    ptr: *const u8,
    len: usize,
}

/// Entry point. Returns a clean `Err` on any mismatch; never panics.
#[allow(dead_code)]
pub fn call_foreign(sym: &ForeignSymbol, args: &[Value]) -> RResult<Value> {
    if args.len() != sym.sig.params.len() {
        return Err(format!(
            "FFI: arity mismatch calling `{}`: expected {}, got {}",
            sym.name,
            sym.sig.params.len(),
            args.len()
        ));
    }

    // RES-216: reject Callback arguments with a clean, discoverable error.
    // Phase 1 recognises the type in extern signatures but cannot yet
    // construct a stable C function pointer from a Resilient closure.
    if sym.sig.params.contains(&FfiType::Callback) || sym.sig.ret == FfiType::Callback {
        return Err(FfiError::CallbackNotYetSupported {
            name: sym.name.clone(),
        }
        .to_string());
    }

    for (i, (arg, want)) in args.iter().zip(sym.sig.params.iter()).enumerate() {
        let actual = ffi_type_of_value(arg);
        if actual != Some(*want) {
            return Err(format!(
                "FFI: type mismatch calling `{}` arg #{}: expected {:?}, got {:?}",
                sym.name, i, want, arg
            ));
        }
    }

    dispatch_explicit(sym, args)
}

#[allow(dead_code)]
fn ffi_type_of_value(v: &Value) -> Option<FfiType> {
    match v {
        Value::Int(_) => Some(FfiType::Int),
        Value::Float(_) => Some(FfiType::Float),
        Value::Bool(_) => Some(FfiType::Bool),
        Value::String(_) => Some(FfiType::Str),
        Value::OpaquePtr(_) => Some(FfiType::OpaquePtr),
        _ => None,
    }
}

#[allow(dead_code)]
fn dispatch_explicit(sym: &ForeignSymbol, args: &[Value]) -> RResult<Value> {
    use FfiType::*;
    let params: &[FfiType] = &sym.sig.params;
    let ret = sym.sig.ret;

    // Extract scalars up front.
    let mut ints: [i64; 8] = [0; 8];
    let mut floats: [f64; 8] = [0.0; 8];
    let mut bools: [bool; 8] = [false; 8];
    let mut strs: [CStr; 8] = [CStr {
        ptr: std::ptr::null(),
        len: 0,
    }; 8];
    // RES-215: opaque pointers are pass-through — no allocation, no
    // marshalling. We just ferry the address across the ABI.
    let mut ptrs: [*mut core::ffi::c_void; 8] = [core::ptr::null_mut(); 8];
    // Keep string byte borrows live for the call.
    let mut live_strs: Vec<&[u8]> = Vec::with_capacity(args.len());
    for (i, (arg, want)) in args.iter().zip(params.iter()).enumerate() {
        match (arg, want) {
            (Value::Int(v), Int) => ints[i] = *v,
            (Value::Float(v), Float) => floats[i] = *v,
            (Value::Bool(v), Bool) => bools[i] = *v,
            (Value::String(s), Str) => {
                let bytes = s.as_bytes();
                live_strs.push(bytes);
                strs[i] = CStr {
                    ptr: bytes.as_ptr(),
                    len: bytes.len(),
                };
            }
            (Value::OpaquePtr(h), OpaquePtr) => ptrs[i] = h.0,
            _ => {
                return Err(format!(
                    "FFI internal: arg #{} type {:?} / ffi {:?} mismatch",
                    i, arg, want
                ));
            }
        }
    }

    // SAFETY: the ForeignSymbol pointer was produced by `libloading`
    // loading a library with the signature declared by the matching
    // `extern` block; the typechecker already gated on primitive-only
    // types so the transmuted fn pointer matches the C ABI. The
    // `live_strs` vector keeps any borrowed String bytes alive across
    // the call.
    let out = unsafe {
        match (params, ret) {
            // ---- Arity 0 ----
            (&[], Int) => Value::Int(std::mem::transmute::<*const (), extern "C" fn() -> i64>(
                sym.ptr,
            )()),
            (&[], Float) => Value::Float(std::mem::transmute::<*const (), extern "C" fn() -> f64>(
                sym.ptr,
            )()),
            (&[], Bool) => Value::Bool(std::mem::transmute::<*const (), extern "C" fn() -> bool>(
                sym.ptr,
            )()),
            (&[], Void) => {
                std::mem::transmute::<*const (), extern "C" fn()>(sym.ptr)();
                Value::Void
            }

            // ---- Arity 1 ----
            (&[Int], Int) => Value::Int(
                std::mem::transmute::<*const (), extern "C" fn(i64) -> i64>(sym.ptr)(ints[0]),
            ),
            (&[Int], Float) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn(i64) -> f64,
            >(sym.ptr)(ints[0])),
            (&[Int], Bool) => Value::Bool(std::mem::transmute::<
                *const (),
                extern "C" fn(i64) -> bool,
            >(sym.ptr)(ints[0])),
            (&[Int], Void) => {
                std::mem::transmute::<*const (), extern "C" fn(i64)>(sym.ptr)(ints[0]);
                Value::Void
            }
            (&[Float], Int) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn(f64) -> i64,
            >(sym.ptr)(floats[0])),
            (&[Float], Float) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn(f64) -> f64,
            >(sym.ptr)(floats[0])),
            (&[Float], Bool) => Value::Bool(std::mem::transmute::<
                *const (),
                extern "C" fn(f64) -> bool,
            >(sym.ptr)(floats[0])),
            (&[Float], Void) => {
                std::mem::transmute::<*const (), extern "C" fn(f64)>(sym.ptr)(floats[0]);
                Value::Void
            }
            (&[Bool], Int) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn(bool) -> i64,
            >(sym.ptr)(bools[0])),
            (&[Bool], Bool) => Value::Bool(std::mem::transmute::<
                *const (),
                extern "C" fn(bool) -> bool,
            >(sym.ptr)(bools[0])),
            (&[Bool], Void) => {
                std::mem::transmute::<*const (), extern "C" fn(bool)>(sym.ptr)(bools[0]);
                Value::Void
            }

            // ---- RES-215: OpaquePtr arms (arity 0 and 1) ----
            (&[], OpaquePtr) => Value::OpaquePtr(crate::ffi::OpaquePtrHandle(
                std::mem::transmute::<*const (), extern "C" fn() -> *mut core::ffi::c_void>(
                    sym.ptr,
                )(),
            )),
            (&[OpaquePtr], OpaquePtr) => Value::OpaquePtr(crate::ffi::OpaquePtrHandle(
                std::mem::transmute::<
                    *const (),
                    extern "C" fn(*mut core::ffi::c_void) -> *mut core::ffi::c_void,
                >(sym.ptr)(ptrs[0]),
            )),
            (&[OpaquePtr], Int) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn(*mut core::ffi::c_void) -> i64,
            >(sym.ptr)(ptrs[0])),
            (&[OpaquePtr], Float) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn(*mut core::ffi::c_void) -> f64,
            >(sym.ptr)(ptrs[0])),
            (&[OpaquePtr], Bool) => Value::Bool(std::mem::transmute::<
                *const (),
                extern "C" fn(*mut core::ffi::c_void) -> bool,
            >(sym.ptr)(ptrs[0])),
            (&[OpaquePtr], Void) => {
                std::mem::transmute::<*const (), extern "C" fn(*mut core::ffi::c_void)>(sym.ptr)(
                    ptrs[0],
                );
                Value::Void
            }

            // ---- Arity 2 (minimal — extend when examples need more) ----
            (&[Float, Float], Float) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn(f64, f64) -> f64,
            >(sym.ptr)(floats[0], floats[1])),
            (&[Int, Int], Int) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn(i64, i64) -> i64,
            >(sym.ptr)(ints[0], ints[1])),

            // Fallback.
            _ => {
                return Err(format!(
                    "FFI: no trampoline for signature ({:?}) -> {:?} (extend dispatch_explicit)",
                    params, ret
                ));
            }
        }
    };

    // `live_strs` and `strs` intentionally outlive the unsafe block
    // via normal scope rules. Touch them here so optimizers can't
    // shuffle the drop earlier than the call.
    drop(live_strs);
    let _ = strs;
    let _ = ptrs;

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::{FfiType, ForeignSignature, ForeignSymbol};

    // A real extern "C" fn we can take the address of for a clean
    // integration test without loading a shared library.
    extern "C" fn sum_two_ints(a: i64, b: i64) -> i64 {
        a + b
    }

    extern "C" fn identity_bool(b: bool) -> bool {
        b
    }

    #[test]
    fn call_foreign_dispatches_two_int_to_int() {
        let sig = ForeignSignature {
            params: vec![FfiType::Int, FfiType::Int],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "sum_two_ints".to_string(),
            ptr: sum_two_ints as *const (),
            sig,
        };
        let out = call_foreign(&sym, &[Value::Int(40), Value::Int(2)]).unwrap();
        assert!(matches!(out, Value::Int(42)), "got {:?}", out);
    }

    #[test]
    fn call_foreign_arity_mismatch_errors_cleanly() {
        let sig = ForeignSignature {
            params: vec![FfiType::Int, FfiType::Int],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "sum_two_ints".to_string(),
            ptr: sum_two_ints as *const (),
            sig,
        };
        let err = call_foreign(&sym, &[Value::Int(1)]).expect_err("should fail");
        assert!(err.contains("arity mismatch"), "got {}", err);
    }

    #[test]
    fn call_foreign_type_mismatch_errors_cleanly() {
        let sig = ForeignSignature {
            params: vec![FfiType::Bool],
            ret: FfiType::Bool,
        };
        let sym = ForeignSymbol {
            name: "identity_bool".to_string(),
            ptr: identity_bool as *const (),
            sig,
        };
        let err = call_foreign(&sym, &[Value::Int(1)]).expect_err("should fail");
        assert!(err.contains("type mismatch"), "got {}", err);
    }

    // RES-215: extern "C" helpers for OpaquePtr round-trip.
    extern "C" fn id_ptr(p: *mut core::ffi::c_void) -> *mut core::ffi::c_void {
        p
    }

    extern "C" fn ptr_addr(p: *mut core::ffi::c_void) -> i64 {
        p as usize as i64
    }

    extern "C" fn make_ptr() -> *mut core::ffi::c_void {
        // Arbitrary non-null sentinel — the language never deref's it.
        0xDEAD_BEEF_usize as *mut core::ffi::c_void
    }

    #[test]
    fn call_foreign_opaque_ptr_round_trip() {
        use crate::ffi::OpaquePtrHandle;
        // (OpaquePtr) -> OpaquePtr: hand the pointer in, get it back.
        let sig = ForeignSignature {
            params: vec![FfiType::OpaquePtr],
            ret: FfiType::OpaquePtr,
        };
        let sym = ForeignSymbol {
            name: "id_ptr".to_string(),
            ptr: id_ptr as *const (),
            sig,
        };
        let sentinel = 0x1234_5678_usize as *mut core::ffi::c_void;
        let out =
            call_foreign(&sym, &[Value::OpaquePtr(OpaquePtrHandle(sentinel))]).expect("ok");
        match out {
            Value::OpaquePtr(h) => assert_eq!(h.0, sentinel),
            other => panic!("expected OpaquePtr, got {:?}", other),
        }
    }

    #[test]
    fn call_foreign_opaque_ptr_to_int() {
        use crate::ffi::OpaquePtrHandle;
        // (OpaquePtr) -> Int: inspect address via the C side.
        let sig = ForeignSignature {
            params: vec![FfiType::OpaquePtr],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "ptr_addr".to_string(),
            ptr: ptr_addr as *const (),
            sig,
        };
        let sentinel = 0x42_usize as *mut core::ffi::c_void;
        let out =
            call_foreign(&sym, &[Value::OpaquePtr(OpaquePtrHandle(sentinel))]).expect("ok");
        assert!(matches!(out, Value::Int(0x42)), "got {:?}", out);
    }

    #[test]
    fn call_foreign_zero_arg_returns_opaque_ptr() {
        let sig = ForeignSignature {
            params: vec![],
            ret: FfiType::OpaquePtr,
        };
        let sym = ForeignSymbol {
            name: "make_ptr".to_string(),
            ptr: make_ptr as *const (),
            sig,
        };
        let out = call_foreign(&sym, &[]).expect("ok");
        match out {
            Value::OpaquePtr(h) => assert_eq!(h.0 as usize, 0xDEAD_BEEF),
            other => panic!("expected OpaquePtr, got {:?}", other),
        }
    }

    #[test]
    fn call_foreign_rejects_callback_argument_as_phase_1_stub() {
        // RES-216: calling an extern fn that declared a `Callback`
        // parameter must return a clean, documented error — not
        // silently do nothing and not panic.
        let sig = ForeignSignature {
            params: vec![FfiType::Callback],
            ret: FfiType::Void,
        };
        let sym = ForeignSymbol {
            name: "register_handler".to_string(),
            // Pointer value is irrelevant; we must short-circuit before dispatch.
            ptr: sum_two_ints as *const (),
            sig,
        };
        // Pass a Resilient Int as a placeholder for the callback value;
        // the Callback branch must fire before type-matching happens.
        let err =
            call_foreign(&sym, &[Value::Int(0)]).expect_err("Callback arg must be rejected in v1");
        assert!(
            err.contains("Callback") && err.contains("Phase 2"),
            "got {}",
            err
        );
        assert!(err.contains("register_handler"), "got {}", err);
    }

    #[test]
    fn call_foreign_missing_arm_errors_cleanly() {
        // Arity 3 is not in the dispatch table yet — must error, not panic.
        let sig = ForeignSignature {
            params: vec![FfiType::Int, FfiType::Int, FfiType::Int],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "bogus".to_string(),
            ptr: sum_two_ints as *const (),
            sig,
        };
        let err = call_foreign(&sym, &[Value::Int(1), Value::Int(2), Value::Int(3)])
            .expect_err("arity 3 has no trampoline");
        assert!(err.contains("no trampoline"), "got {}", err);
    }
}
