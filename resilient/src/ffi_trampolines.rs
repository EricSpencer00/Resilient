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
use crate::ffi::{FfiType, ForeignSymbol};
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
