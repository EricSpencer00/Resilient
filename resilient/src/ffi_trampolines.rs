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
//!
//! RES-317: in addition to scalars, the trampoline now bridges
//! `@repr(C)` structs that fit in a single 8-byte register (one
//! SystemV INTEGER class). The marshaller packs the Resilient struct's
//! field values into a stack-allocated `u64` buffer matching the C
//! struct's layout, hands it to the C function as a by-value
//! integer, and on return unpacks the returned u64 back into a
//! `Value::Struct`. Anything larger than 8 bytes returns a clean
//! `FfiError::StructTooLarge` rather than calling the function with
//! the wrong ABI.
use crate::ffi::{FfiError, FfiType, ForeignSymbol, struct_layout};
use crate::{RResult, Value};

#[allow(dead_code)]
#[derive(Copy, Clone)]
#[repr(C)]
struct CStr {
    ptr: *const u8,
    len: usize,
}

/// RES-317: maximum size in bytes for a struct passed/returned by
/// value through the Phase 1 trampoline. Matches the SystemV
/// INTEGER class single-register limit on x86_64 / AArch64. Larger
/// structs need an out-pointer convention or libffi — tracked as
/// a follow-up.
const STRUCT_BY_VALUE_MAX: usize = 8;

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
    if sym
        .sig
        .params
        .iter()
        .any(|t| matches!(t, FfiType::Callback))
        || matches!(sym.sig.ret, FfiType::Callback)
    {
        return Err(FfiError::CallbackNotYetSupported {
            name: sym.name.clone(),
        }
        .to_string());
    }

    for (i, (arg, want)) in args.iter().zip(sym.sig.params.iter()).enumerate() {
        if !value_matches_ffi_type(arg, want) {
            return Err(format!(
                "FFI: type mismatch calling `{}` arg #{}: expected {:?}, got {:?}",
                sym.name, i, want, arg
            ));
        }
    }

    dispatch_explicit(sym, args)
}

#[allow(dead_code)]
fn value_matches_ffi_type(v: &Value, want: &FfiType) -> bool {
    match (v, want) {
        (Value::Int(_), FfiType::Int) => true,
        (Value::Float(_), FfiType::Float) => true,
        (Value::Bool(_), FfiType::Bool) => true,
        (Value::String(_), FfiType::Str) => true,
        (Value::OpaquePtr(_), FfiType::OpaquePtr) => true,
        // RES-317: `Value::Struct` matches `FfiType::Struct` only when
        // the declared name and field count line up. Per-field type
        // checking happens in the marshaller so a mismatch yields a
        // pinpoint diagnostic.
        (
            Value::Struct {
                name: vname,
                fields: vfields,
            },
            FfiType::Struct {
                name: tname,
                fields: tfields,
            },
        ) => vname == tname && vfields.len() == tfields.len(),
        _ => false,
    }
}

/// RES-317: pack a `Value::Struct` into a `[u8; 8]` buffer that
/// matches the C struct layout for `ty`. `ty` must be `FfiType::Struct`
/// and total size must be ≤ 8. Returns the buffer interpreted as a
/// little-endian `u64` so it can be passed as an INTEGER-class
/// argument on x86_64 / AArch64.
fn pack_struct_to_u64(value: &Value, ty: &FfiType) -> Result<u64, String> {
    let (sname, sfields) = match ty {
        FfiType::Struct { name, fields } => (name, fields),
        _ => {
            return Err("FFI internal: pack_struct_to_u64 called with non-struct type".to_string());
        }
    };
    let layout = struct_layout(sfields);
    if layout.total > STRUCT_BY_VALUE_MAX {
        return Err(FfiError::StructTooLarge {
            name: sname.clone(),
            size: layout.total,
            max: STRUCT_BY_VALUE_MAX,
        }
        .to_string());
    }
    let (vname, vfields) = match value {
        Value::Struct { name, fields } => (name, fields),
        other => {
            return Err(format!("FFI: expected struct `{}`, got {:?}", sname, other));
        }
    };
    if vname != sname {
        return Err(format!(
            "FFI: expected struct `{}`, got struct `{}`",
            sname, vname
        ));
    }

    let mut buf: [u8; STRUCT_BY_VALUE_MAX] = [0; STRUCT_BY_VALUE_MAX];
    for (i, (fname, fty)) in sfields.iter().enumerate() {
        let v = vfields
            .iter()
            .find_map(|(n, val)| if n == fname { Some(val) } else { None })
            .ok_or_else(|| {
                format!(
                    "FFI: struct `{}` is missing field `{}` required by extern signature",
                    sname, fname
                )
            })?;
        let off = layout.offsets[i];
        write_field(&mut buf, off, v, fty)
            .map_err(|e| format!("FFI: struct `{}`.`{}`: {}", sname, fname, e))?;
    }
    Ok(u64::from_le_bytes(buf))
}

/// RES-317: unpack a `u64` returned by C as an INTEGER-class struct
/// back into a Resilient `Value::Struct`. `ty` must be
/// `FfiType::Struct` with total size ≤ 8.
fn unpack_struct_from_u64(raw: u64, ty: &FfiType) -> Result<Value, String> {
    let (sname, sfields) = match ty {
        FfiType::Struct { name, fields } => (name, fields),
        _ => {
            return Err(
                "FFI internal: unpack_struct_from_u64 called with non-struct type".to_string(),
            );
        }
    };
    let layout = struct_layout(sfields);
    if layout.total > STRUCT_BY_VALUE_MAX {
        return Err(FfiError::StructTooLarge {
            name: sname.clone(),
            size: layout.total,
            max: STRUCT_BY_VALUE_MAX,
        }
        .to_string());
    }
    let buf: [u8; STRUCT_BY_VALUE_MAX] = raw.to_le_bytes();
    let mut fields: Vec<(String, Value)> = Vec::with_capacity(sfields.len());
    for (i, (fname, fty)) in sfields.iter().enumerate() {
        let off = layout.offsets[i];
        let v = read_field(&buf, off, fty)
            .map_err(|e| format!("FFI: struct `{}`.`{}`: {}", sname, fname, e))?;
        fields.push((fname.clone(), v));
    }
    Ok(Value::Struct {
        name: sname.clone(),
        fields,
    })
}

/// Write one field's bytes into `buf` at `offset`. Bool/Int/Float are
/// the only field types Phase 1 supports inside structs; nested
/// structs / strings / pointers are out of scope and return an error.
fn write_field(buf: &mut [u8], offset: usize, v: &Value, fty: &FfiType) -> Result<(), String> {
    match (fty, v) {
        (FfiType::Int, Value::Int(i)) => {
            // Resilient `Int` is i64 internally; C `int64_t` matches.
            let bytes = i.to_le_bytes();
            buf[offset..offset + 8].copy_from_slice(&bytes);
            Ok(())
        }
        (FfiType::Float, Value::Float(f)) => {
            let bytes = f.to_le_bytes();
            buf[offset..offset + 8].copy_from_slice(&bytes);
            Ok(())
        }
        (FfiType::Bool, Value::Bool(b)) => {
            buf[offset] = u8::from(*b);
            Ok(())
        }
        // Phase 1: only scalar fields are supported. Strings, opaque
        // pointers, callbacks and nested structs need a layout policy
        // we haven't shipped yet.
        (FfiType::Int, _) | (FfiType::Float, _) | (FfiType::Bool, _) => Err(format!(
            "type mismatch: declared as {:?}, value is {:?}",
            fty, v
        )),
        (other, _) => Err(format!(
            "field type {:?} is not supported inside `@repr(C)` structs in Phase 1",
            other
        )),
    }
}

fn read_field(buf: &[u8], offset: usize, fty: &FfiType) -> Result<Value, String> {
    match fty {
        FfiType::Int => {
            let mut bs = [0u8; 8];
            bs.copy_from_slice(&buf[offset..offset + 8]);
            Ok(Value::Int(i64::from_le_bytes(bs)))
        }
        FfiType::Float => {
            let mut bs = [0u8; 8];
            bs.copy_from_slice(&buf[offset..offset + 8]);
            Ok(Value::Float(f64::from_le_bytes(bs)))
        }
        FfiType::Bool => Ok(Value::Bool(buf[offset] != 0)),
        other => Err(format!(
            "field type {:?} is not supported inside `@repr(C)` structs in Phase 1",
            other
        )),
    }
}

#[allow(dead_code)]
fn dispatch_explicit(sym: &ForeignSymbol, args: &[Value]) -> RResult<Value> {
    let params: &[FfiType] = &sym.sig.params;
    let ret: &FfiType = &sym.sig.ret;

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
    // RES-317: packed integer-class struct values, indexed by arg
    // position. `0` is a meaningless default for non-struct slots.
    let mut struct_words: [u64; 8] = [0; 8];
    // Keep string byte borrows live for the call.
    let mut live_strs: Vec<&[u8]> = Vec::with_capacity(args.len());
    for (i, (arg, want)) in args.iter().zip(params.iter()).enumerate() {
        match (arg, want) {
            (Value::Int(v), FfiType::Int) => ints[i] = *v,
            (Value::Float(v), FfiType::Float) => floats[i] = *v,
            (Value::Bool(v), FfiType::Bool) => bools[i] = *v,
            (Value::String(s), FfiType::Str) => {
                let bytes = s.as_bytes();
                live_strs.push(bytes);
                strs[i] = CStr {
                    ptr: bytes.as_ptr(),
                    len: bytes.len(),
                };
            }
            (Value::OpaquePtr(h), FfiType::OpaquePtr) => ptrs[i] = h.0,
            (Value::Struct { .. }, FfiType::Struct { .. }) => {
                struct_words[i] = pack_struct_to_u64(arg, want)?;
            }
            _ => {
                return Err(format!(
                    "FFI internal: arg #{} type {:?} / ffi {:?} mismatch",
                    i, arg, want
                ));
            }
        }
    }

    // RES-317: dispatch struct-bearing signatures separately. They live
    // outside the giant scalar-arms match so we can keep the scalar table
    // unchanged and re-use the packed `u64` representation.
    if let Some(out) = dispatch_struct_signatures(sym, params, ret, &ints, &struct_words)? {
        drop(live_strs);
        let _ = strs;
        let _ = ptrs;
        let _ = struct_words;
        return Ok(out);
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
            (&[], FfiType::Int) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn() -> i64,
            >(sym.ptr)()),
            (&[], FfiType::Float) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn() -> f64,
            >(sym.ptr)()),
            (&[], FfiType::Bool) => Value::Bool(std::mem::transmute::<
                *const (),
                extern "C" fn() -> bool,
            >(sym.ptr)()),
            (&[], FfiType::Void) => {
                std::mem::transmute::<*const (), extern "C" fn()>(sym.ptr)();
                Value::Void
            }

            // ---- Arity 1 ----
            ([FfiType::Int], FfiType::Int) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn(i64) -> i64,
            >(sym.ptr)(ints[0])),
            ([FfiType::Int], FfiType::Float) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn(i64) -> f64,
            >(sym.ptr)(ints[0])),
            ([FfiType::Int], FfiType::Bool) => Value::Bool(std::mem::transmute::<
                *const (),
                extern "C" fn(i64) -> bool,
            >(sym.ptr)(ints[0])),
            ([FfiType::Int], FfiType::Void) => {
                std::mem::transmute::<*const (), extern "C" fn(i64)>(sym.ptr)(ints[0]);
                Value::Void
            }
            ([FfiType::Float], FfiType::Int) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn(f64) -> i64,
            >(sym.ptr)(floats[0])),
            ([FfiType::Float], FfiType::Float) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn(f64) -> f64,
            >(sym.ptr)(floats[0])),
            ([FfiType::Float], FfiType::Bool) => Value::Bool(std::mem::transmute::<
                *const (),
                extern "C" fn(f64) -> bool,
            >(sym.ptr)(floats[0])),
            ([FfiType::Float], FfiType::Void) => {
                std::mem::transmute::<*const (), extern "C" fn(f64)>(sym.ptr)(floats[0]);
                Value::Void
            }
            ([FfiType::Bool], FfiType::Int) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn(bool) -> i64,
            >(sym.ptr)(bools[0])),
            ([FfiType::Bool], FfiType::Bool) => Value::Bool(std::mem::transmute::<
                *const (),
                extern "C" fn(bool) -> bool,
            >(sym.ptr)(bools[0])),
            ([FfiType::Bool], FfiType::Void) => {
                std::mem::transmute::<*const (), extern "C" fn(bool)>(sym.ptr)(bools[0]);
                Value::Void
            }

            // ---- RES-215: OpaquePtr arms (arity 0 and 1) ----
            (&[], FfiType::OpaquePtr) => {
                Value::OpaquePtr(crate::ffi::OpaquePtrHandle(std::mem::transmute::<
                    *const (),
                    extern "C" fn() -> *mut core::ffi::c_void,
                >(sym.ptr)()))
            }
            ([FfiType::OpaquePtr], FfiType::OpaquePtr) => {
                Value::OpaquePtr(crate::ffi::OpaquePtrHandle(std::mem::transmute::<
                    *const (),
                    extern "C" fn(*mut core::ffi::c_void) -> *mut core::ffi::c_void,
                >(sym.ptr)(
                    ptrs[0]
                )))
            }
            ([FfiType::OpaquePtr], FfiType::Int) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn(*mut core::ffi::c_void) -> i64,
            >(sym.ptr)(ptrs[0])),
            ([FfiType::OpaquePtr], FfiType::Float) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn(*mut core::ffi::c_void) -> f64,
            >(sym.ptr)(ptrs[0])),
            ([FfiType::OpaquePtr], FfiType::Bool) => Value::Bool(std::mem::transmute::<
                *const (),
                extern "C" fn(*mut core::ffi::c_void) -> bool,
            >(sym.ptr)(ptrs[0])),
            ([FfiType::OpaquePtr], FfiType::Void) => {
                std::mem::transmute::<*const (), extern "C" fn(*mut core::ffi::c_void)>(sym.ptr)(
                    ptrs[0],
                );
                Value::Void
            }

            // ---- Arity 2 (minimal — extend when examples need more) ----
            ([FfiType::Float, FfiType::Float], FfiType::Float) => {
                Value::Float(std::mem::transmute::<
                    *const (),
                    extern "C" fn(f64, f64) -> f64,
                >(sym.ptr)(floats[0], floats[1]))
            }
            ([FfiType::Int, FfiType::Int], FfiType::Int) => {
                Value::Int(std::mem::transmute::<
                    *const (),
                    extern "C" fn(i64, i64) -> i64,
                >(sym.ptr)(ints[0], ints[1]))
            }

            // ---- Arity 2 (missing arms) ----
            ([FfiType::Int, FfiType::Int], FfiType::Void) => {
                std::mem::transmute::<*const (), extern "C" fn(i64, i64)>(sym.ptr)(
                    ints[0], ints[1],
                );
                Value::Void
            }
            ([FfiType::Int, FfiType::Int], FfiType::Float) => {
                Value::Float(std::mem::transmute::<
                    *const (),
                    extern "C" fn(i64, i64) -> f64,
                >(sym.ptr)(ints[0], ints[1]))
            }
            ([FfiType::Float, FfiType::Float], FfiType::Void) => {
                std::mem::transmute::<*const (), extern "C" fn(f64, f64)>(sym.ptr)(
                    floats[0], floats[1],
                );
                Value::Void
            }
            ([FfiType::Float, FfiType::Float], FfiType::Int) => {
                Value::Int(std::mem::transmute::<
                    *const (),
                    extern "C" fn(f64, f64) -> i64,
                >(sym.ptr)(floats[0], floats[1]))
            }
            ([FfiType::OpaquePtr, FfiType::Int], FfiType::Void) => {
                std::mem::transmute::<*const (), extern "C" fn(*mut core::ffi::c_void, i64)>(
                    sym.ptr,
                )(ptrs[0], ints[1]);
                Value::Void
            }
            ([FfiType::Int, FfiType::OpaquePtr], FfiType::OpaquePtr) => {
                Value::OpaquePtr(crate::ffi::OpaquePtrHandle(std::mem::transmute::<
                    *const (),
                    extern "C" fn(i64, *mut core::ffi::c_void) -> *mut core::ffi::c_void,
                >(sym.ptr)(
                    ints[0], ptrs[1]
                )))
            }

            // ---- Arity 3 ----
            ([FfiType::Int, FfiType::Int, FfiType::Int], FfiType::Int) => {
                Value::Int(std::mem::transmute::<
                    *const (),
                    extern "C" fn(i64, i64, i64) -> i64,
                >(sym.ptr)(ints[0], ints[1], ints[2]))
            }
            ([FfiType::Int, FfiType::Int, FfiType::Int], FfiType::Void) => {
                std::mem::transmute::<*const (), extern "C" fn(i64, i64, i64)>(sym.ptr)(
                    ints[0], ints[1], ints[2],
                );
                Value::Void
            }
            ([FfiType::Int, FfiType::Int, FfiType::Int], FfiType::Float) => {
                Value::Float(std::mem::transmute::<
                    *const (),
                    extern "C" fn(i64, i64, i64) -> f64,
                >(sym.ptr)(ints[0], ints[1], ints[2]))
            }
            ([FfiType::Float, FfiType::Float, FfiType::Float], FfiType::Float) => {
                Value::Float(std::mem::transmute::<
                    *const (),
                    extern "C" fn(f64, f64, f64) -> f64,
                >(sym.ptr)(floats[0], floats[1], floats[2]))
            }
            ([FfiType::Float, FfiType::Float, FfiType::Float], FfiType::Void) => {
                std::mem::transmute::<*const (), extern "C" fn(f64, f64, f64)>(sym.ptr)(
                    floats[0], floats[1], floats[2],
                );
                Value::Void
            }
            ([FfiType::Int, FfiType::Float, FfiType::Float], FfiType::Float) => {
                Value::Float(std::mem::transmute::<
                    *const (),
                    extern "C" fn(i64, f64, f64) -> f64,
                >(sym.ptr)(ints[0], floats[1], floats[2]))
            }
            ([FfiType::OpaquePtr, FfiType::Int, FfiType::Int], FfiType::Int) => {
                Value::Int(std::mem::transmute::<
                    *const (),
                    extern "C" fn(*mut core::ffi::c_void, i64, i64) -> i64,
                >(sym.ptr)(ptrs[0], ints[1], ints[2]))
            }
            ([FfiType::OpaquePtr, FfiType::Int, FfiType::Int], FfiType::Void) => {
                std::mem::transmute::<*const (), extern "C" fn(*mut core::ffi::c_void, i64, i64)>(
                    sym.ptr,
                )(ptrs[0], ints[1], ints[2]);
                Value::Void
            }

            // ---- RES-FFI-V3: Arity 4 ----
            ([FfiType::Int, FfiType::Int, FfiType::Int, FfiType::Int], FfiType::Int) => {
                Value::Int(std::mem::transmute::<
                    *const (),
                    extern "C" fn(i64, i64, i64, i64) -> i64,
                >(sym.ptr)(
                    ints[0], ints[1], ints[2], ints[3]
                ))
            }
            ([FfiType::Int, FfiType::Int, FfiType::Int, FfiType::Int], FfiType::Void) => {
                std::mem::transmute::<*const (), extern "C" fn(i64, i64, i64, i64)>(sym.ptr)(
                    ints[0], ints[1], ints[2], ints[3],
                );
                Value::Void
            }
            (
                [
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                ],
                FfiType::Float,
            ) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn(f64, f64, f64, f64) -> f64,
            >(sym.ptr)(
                floats[0], floats[1], floats[2], floats[3]
            )),
            (
                [
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                ],
                FfiType::Void,
            ) => {
                std::mem::transmute::<*const (), extern "C" fn(f64, f64, f64, f64)>(sym.ptr)(
                    floats[0], floats[1], floats[2], floats[3],
                );
                Value::Void
            }

            // ---- RES-FFI-V3: Arity 5 ----
            (
                [
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                ],
                FfiType::Int,
            ) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn(i64, i64, i64, i64, i64) -> i64,
            >(sym.ptr)(
                ints[0], ints[1], ints[2], ints[3], ints[4]
            )),
            (
                [
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                ],
                FfiType::Void,
            ) => {
                std::mem::transmute::<*const (), extern "C" fn(i64, i64, i64, i64, i64)>(sym.ptr)(
                    ints[0], ints[1], ints[2], ints[3], ints[4],
                );
                Value::Void
            }
            (
                [
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                ],
                FfiType::Float,
            ) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn(f64, f64, f64, f64, f64) -> f64,
            >(sym.ptr)(
                floats[0], floats[1], floats[2], floats[3], floats[4]
            )),
            (
                [
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                ],
                FfiType::Void,
            ) => {
                std::mem::transmute::<*const (), extern "C" fn(f64, f64, f64, f64, f64)>(sym.ptr)(
                    floats[0], floats[1], floats[2], floats[3], floats[4],
                );
                Value::Void
            }

            // ---- RES-FFI-V3: Arity 6 ----
            (
                [
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                ],
                FfiType::Int,
            ) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn(i64, i64, i64, i64, i64, i64) -> i64,
            >(sym.ptr)(
                ints[0], ints[1], ints[2], ints[3], ints[4], ints[5]
            )),
            (
                [
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                ],
                FfiType::Void,
            ) => {
                std::mem::transmute::<*const (), extern "C" fn(i64, i64, i64, i64, i64, i64)>(
                    sym.ptr,
                )(ints[0], ints[1], ints[2], ints[3], ints[4], ints[5]);
                Value::Void
            }
            (
                [
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                ],
                FfiType::Float,
            ) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn(f64, f64, f64, f64, f64, f64) -> f64,
            >(sym.ptr)(
                floats[0], floats[1], floats[2], floats[3], floats[4], floats[5],
            )),
            (
                [
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                ],
                FfiType::Void,
            ) => {
                std::mem::transmute::<*const (), extern "C" fn(f64, f64, f64, f64, f64, f64)>(
                    sym.ptr,
                )(
                    floats[0], floats[1], floats[2], floats[3], floats[4], floats[5],
                );
                Value::Void
            }

            // ---- RES-FFI-V3: Arity 7 ----
            (
                [
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                ],
                FfiType::Int,
            ) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn(i64, i64, i64, i64, i64, i64, i64) -> i64,
            >(sym.ptr)(
                ints[0], ints[1], ints[2], ints[3], ints[4], ints[5], ints[6],
            )),
            (
                [
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                ],
                FfiType::Void,
            ) => {
                std::mem::transmute::<*const (), extern "C" fn(i64, i64, i64, i64, i64, i64, i64)>(
                    sym.ptr,
                )(
                    ints[0], ints[1], ints[2], ints[3], ints[4], ints[5], ints[6],
                );
                Value::Void
            }
            (
                [
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                ],
                FfiType::Float,
            ) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn(f64, f64, f64, f64, f64, f64, f64) -> f64,
            >(sym.ptr)(
                floats[0], floats[1], floats[2], floats[3], floats[4], floats[5], floats[6],
            )),
            (
                [
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                ],
                FfiType::Void,
            ) => {
                std::mem::transmute::<*const (), extern "C" fn(f64, f64, f64, f64, f64, f64, f64)>(
                    sym.ptr,
                )(
                    floats[0], floats[1], floats[2], floats[3], floats[4], floats[5], floats[6],
                );
                Value::Void
            }

            // ---- RES-FFI-V3: Arity 8 ----
            (
                [
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                ],
                FfiType::Int,
            ) => Value::Int(std::mem::transmute::<
                *const (),
                extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64) -> i64,
            >(sym.ptr)(
                ints[0], ints[1], ints[2], ints[3], ints[4], ints[5], ints[6], ints[7],
            )),
            (
                [
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                    FfiType::Int,
                ],
                FfiType::Void,
            ) => {
                std::mem::transmute::<
                    *const (),
                    extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64),
                >(sym.ptr)(
                    ints[0], ints[1], ints[2], ints[3], ints[4], ints[5], ints[6], ints[7],
                );
                Value::Void
            }
            (
                [
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                ],
                FfiType::Float,
            ) => Value::Float(std::mem::transmute::<
                *const (),
                extern "C" fn(f64, f64, f64, f64, f64, f64, f64, f64) -> f64,
            >(sym.ptr)(
                floats[0], floats[1], floats[2], floats[3], floats[4], floats[5], floats[6],
                floats[7],
            )),
            (
                [
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                    FfiType::Float,
                ],
                FfiType::Void,
            ) => {
                std::mem::transmute::<
                    *const (),
                    extern "C" fn(f64, f64, f64, f64, f64, f64, f64, f64),
                >(sym.ptr)(
                    floats[0], floats[1], floats[2], floats[3], floats[4], floats[5], floats[6],
                    floats[7],
                );
                Value::Void
            }

            // ---- RES-FFI-V3: Mixed Int+Float patterns (arity 4) ----
            // Common in physics/control: (Float, Float, Float, Int) -> Float
            ([FfiType::Float, FfiType::Float, FfiType::Float, FfiType::Int], FfiType::Float) => {
                Value::Float(std::mem::transmute::<
                    *const (),
                    extern "C" fn(f64, f64, f64, i64) -> f64,
                >(sym.ptr)(
                    floats[0], floats[1], floats[2], ints[3]
                ))
            }
            ([FfiType::Int, FfiType::Float, FfiType::Float, FfiType::Float], FfiType::Float) => {
                Value::Float(std::mem::transmute::<
                    *const (),
                    extern "C" fn(i64, f64, f64, f64) -> f64,
                >(sym.ptr)(
                    ints[0], floats[1], floats[2], floats[3]
                ))
            }

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
    let _ = struct_words;

    Ok(out)
}

/// RES-317: dispatch the small struct signatures we support in
/// Phase 1. Returns `Ok(Some(value))` on a hit, `Ok(None)` if the
/// signature contains no struct (caller falls back to the scalar
/// table), or `Err(...)` on a struct-related error (size, layout, etc.).
///
/// SAFETY: every transmute uses an `extern "C" fn` whose argument /
/// return types match the layout the C compiler would produce for
/// the corresponding `@repr(C)` struct. Per the SystemV / Windows-x64
/// / AArch64 calling conventions, a struct of total size ≤ 8 bytes
/// containing only INTEGER-class fields is passed in a single
/// general-purpose register — i.e. the same register that would
/// carry a `u64` argument. That equivalence is the load-bearing
/// invariant; we explicitly cap struct size at `STRUCT_BY_VALUE_MAX`
/// to enforce it. Float-only / mixed-class structs will hit the
/// `StructTooLarge`-style fallback in a future ticket once we
/// support multi-class lowering.
fn dispatch_struct_signatures(
    sym: &ForeignSymbol,
    params: &[FfiType],
    ret: &FfiType,
    ints: &[i64; 8],
    struct_words: &[u64; 8],
) -> RResult<Option<Value>> {
    // Quick reject: nothing to do unless a struct is on either side.
    let any_struct = matches!(ret, FfiType::Struct { .. })
        || params.iter().any(|p| matches!(p, FfiType::Struct { .. }));
    if !any_struct {
        return Ok(None);
    }

    match (params, ret) {
        // (Struct) -> Struct
        ([FfiType::Struct { .. }], FfiType::Struct { .. }) => {
            // Pre-compute size so we surface a clean StructTooLarge before transmute.
            check_struct_size(&params[0])?;
            check_struct_size(ret)?;
            // SAFETY: see fn doc — size-capped Struct ≡ u64 in INTEGER class.
            let raw = unsafe {
                std::mem::transmute::<*const (), extern "C" fn(u64) -> u64>(sym.ptr)(
                    struct_words[0],
                )
            };
            Ok(Some(unpack_struct_from_u64(raw, ret)?))
        }
        // () -> Struct
        (&[], FfiType::Struct { .. }) => {
            check_struct_size(ret)?;
            let raw =
                unsafe { std::mem::transmute::<*const (), extern "C" fn() -> u64>(sym.ptr)() };
            Ok(Some(unpack_struct_from_u64(raw, ret)?))
        }
        // (Struct) -> Void
        ([FfiType::Struct { .. }], FfiType::Void) => {
            check_struct_size(&params[0])?;
            unsafe {
                std::mem::transmute::<*const (), extern "C" fn(u64)>(sym.ptr)(struct_words[0]);
            }
            Ok(Some(Value::Void))
        }
        // (Struct) -> Int — common readback shape (e.g. `compute_total(reading)`).
        ([FfiType::Struct { .. }], FfiType::Int) => {
            check_struct_size(&params[0])?;
            let v = unsafe {
                std::mem::transmute::<*const (), extern "C" fn(u64) -> i64>(sym.ptr)(
                    struct_words[0],
                )
            };
            Ok(Some(Value::Int(v)))
        }
        // (Int) -> Struct — common factory shape.
        ([FfiType::Int], FfiType::Struct { .. }) => {
            check_struct_size(ret)?;
            let raw = unsafe {
                std::mem::transmute::<*const (), extern "C" fn(i64) -> u64>(sym.ptr)(ints[0])
            };
            Ok(Some(unpack_struct_from_u64(raw, ret)?))
        }
        // Anything else with structs: unsupported in Phase 1.
        _ => Err(format!(
            "FFI: no struct trampoline for signature ({:?}) -> {:?} (extend dispatch_struct_signatures)",
            params, ret
        )),
    }
}

fn check_struct_size(ty: &FfiType) -> Result<(), String> {
    if let FfiType::Struct { name, fields } = ty {
        let layout = struct_layout(fields);
        if layout.total > STRUCT_BY_VALUE_MAX {
            return Err(FfiError::StructTooLarge {
                name: name.clone(),
                size: layout.total,
                max: STRUCT_BY_VALUE_MAX,
            }
            .to_string());
        }
    }
    Ok(())
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
        let out = call_foreign(&sym, &[Value::OpaquePtr(OpaquePtrHandle(sentinel))]).expect("ok");
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
        let out = call_foreign(&sym, &[Value::OpaquePtr(OpaquePtrHandle(sentinel))]).expect("ok");
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
        // RES-FFI-V3: arity 4 Int/Float arms now exist, so use an
        // arity-4 Bool combination — still uncovered — to exercise
        // the fallback error path without panicking on bounds.
        let sig = ForeignSignature {
            params: vec![FfiType::Bool, FfiType::Bool, FfiType::Bool, FfiType::Bool],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "bogus".to_string(),
            ptr: sum_two_ints as *const (),
            sig,
        };
        let err = call_foreign(
            &sym,
            &[
                Value::Bool(true),
                Value::Bool(false),
                Value::Bool(true),
                Value::Bool(false),
            ],
        )
        .expect_err("(Bool,Bool,Bool,Bool) -> Int has no trampoline");
        assert!(err.contains("no trampoline"), "got {}", err);
    }

    extern "C" fn three_ints_sum(a: i64, b: i64, c: i64) -> i64 {
        a + b + c
    }
    extern "C" fn two_ints_void(_a: i64, _b: i64) {}
    extern "C" fn three_floats_sum(a: f64, b: f64, c: f64) -> f64 {
        a + b + c
    }

    #[test]
    fn arity_3_int_int_int_to_int() {
        let sig = ForeignSignature {
            params: vec![FfiType::Int, FfiType::Int, FfiType::Int],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "three_ints_sum".into(),
            ptr: three_ints_sum as *const (),
            sig,
        };
        let out = call_foreign(&sym, &[Value::Int(1), Value::Int(2), Value::Int(3)]).unwrap();
        assert!(matches!(out, Value::Int(6)), "got {:?}", out);
    }

    #[test]
    fn arity_2_int_int_void() {
        let sig = ForeignSignature {
            params: vec![FfiType::Int, FfiType::Int],
            ret: FfiType::Void,
        };
        let sym = ForeignSymbol {
            name: "two_ints_void".into(),
            ptr: two_ints_void as *const (),
            sig,
        };
        let out = call_foreign(&sym, &[Value::Int(1), Value::Int(2)]).unwrap();
        assert!(matches!(out, Value::Void), "got {:?}", out);
    }

    #[test]
    fn arity_3_float_float_float_to_float() {
        let sig = ForeignSignature {
            params: vec![FfiType::Float, FfiType::Float, FfiType::Float],
            ret: FfiType::Float,
        };
        let sym = ForeignSymbol {
            name: "three_floats_sum".into(),
            ptr: three_floats_sum as *const (),
            sig,
        };
        let out = call_foreign(
            &sym,
            &[Value::Float(1.0), Value::Float(2.0), Value::Float(3.0)],
        )
        .unwrap();
        assert!(
            matches!(out, Value::Float(f) if (f - 6.0).abs() < 1e-9),
            "got {:?}",
            out
        );
    }

    // ============================================================
    // RES-FFI-V3: arity 4–8 trampoline coverage.
    // ============================================================

    extern "C" fn sum_4_ints(a: i64, b: i64, c: i64, d: i64) -> i64 {
        a + b + c + d
    }
    extern "C" fn sum_5_ints(a: i64, b: i64, c: i64, d: i64, e: i64) -> i64 {
        a + b + c + d + e
    }
    extern "C" fn sum_6_ints(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64) -> i64 {
        a + b + c + d + e + f
    }
    extern "C" fn sum_7_ints(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64) -> i64 {
        a + b + c + d + e + f + g
    }
    extern "C" fn sum_8_ints(
        a: i64,
        b: i64,
        c: i64,
        d: i64,
        e: i64,
        f: i64,
        g: i64,
        h: i64,
    ) -> i64 {
        a + b + c + d + e + f + g + h
    }
    extern "C" fn sum_4_floats(a: f64, b: f64, c: f64, d: f64) -> f64 {
        a + b + c + d
    }
    extern "C" fn sum_5_floats(a: f64, b: f64, c: f64, d: f64, e: f64) -> f64 {
        a + b + c + d + e
    }
    extern "C" fn sum_6_floats(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> f64 {
        a + b + c + d + e + f
    }
    extern "C" fn sum_7_floats(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64, g: f64) -> f64 {
        a + b + c + d + e + f + g
    }
    extern "C" fn sum_8_floats(
        a: f64,
        b: f64,
        c: f64,
        d: f64,
        e: f64,
        f: f64,
        g: f64,
        h: f64,
    ) -> f64 {
        a + b + c + d + e + f + g + h
    }
    extern "C" fn four_ints_void(_a: i64, _b: i64, _c: i64, _d: i64) {}

    #[test]
    fn arity_4_int_to_int() {
        let sig = ForeignSignature {
            params: vec![FfiType::Int, FfiType::Int, FfiType::Int, FfiType::Int],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "sum_4_ints".into(),
            ptr: sum_4_ints as *const (),
            sig,
        };
        let out = call_foreign(
            &sym,
            &[Value::Int(1), Value::Int(2), Value::Int(3), Value::Int(4)],
        )
        .unwrap();
        assert!(matches!(out, Value::Int(10)), "got {:?}", out);
    }

    #[test]
    fn arity_4_int_to_void() {
        let sig = ForeignSignature {
            params: vec![FfiType::Int, FfiType::Int, FfiType::Int, FfiType::Int],
            ret: FfiType::Void,
        };
        let sym = ForeignSymbol {
            name: "four_ints_void".into(),
            ptr: four_ints_void as *const (),
            sig,
        };
        let out = call_foreign(
            &sym,
            &[Value::Int(1), Value::Int(2), Value::Int(3), Value::Int(4)],
        )
        .unwrap();
        assert!(matches!(out, Value::Void), "got {:?}", out);
    }

    #[test]
    fn arity_4_float_to_float() {
        let sig = ForeignSignature {
            params: vec![
                FfiType::Float,
                FfiType::Float,
                FfiType::Float,
                FfiType::Float,
            ],
            ret: FfiType::Float,
        };
        let sym = ForeignSymbol {
            name: "sum_4_floats".into(),
            ptr: sum_4_floats as *const (),
            sig,
        };
        let out = call_foreign(
            &sym,
            &[
                Value::Float(1.0),
                Value::Float(2.0),
                Value::Float(3.0),
                Value::Float(4.0),
            ],
        )
        .unwrap();
        assert!(
            matches!(out, Value::Float(f) if (f - 10.0).abs() < 1e-9),
            "got {:?}",
            out
        );
    }

    #[test]
    fn arity_5_int_to_int() {
        let sig = ForeignSignature {
            params: vec![
                FfiType::Int,
                FfiType::Int,
                FfiType::Int,
                FfiType::Int,
                FfiType::Int,
            ],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "sum_5_ints".into(),
            ptr: sum_5_ints as *const (),
            sig,
        };
        let out = call_foreign(
            &sym,
            &[
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
                Value::Int(5),
            ],
        )
        .unwrap();
        assert!(matches!(out, Value::Int(15)), "got {:?}", out);
    }

    #[test]
    fn arity_5_float_to_float() {
        let sig = ForeignSignature {
            params: vec![
                FfiType::Float,
                FfiType::Float,
                FfiType::Float,
                FfiType::Float,
                FfiType::Float,
            ],
            ret: FfiType::Float,
        };
        let sym = ForeignSymbol {
            name: "sum_5_floats".into(),
            ptr: sum_5_floats as *const (),
            sig,
        };
        let out = call_foreign(
            &sym,
            &[
                Value::Float(1.0),
                Value::Float(2.0),
                Value::Float(3.0),
                Value::Float(4.0),
                Value::Float(5.0),
            ],
        )
        .unwrap();
        assert!(
            matches!(out, Value::Float(f) if (f - 15.0).abs() < 1e-9),
            "got {:?}",
            out
        );
    }

    #[test]
    fn arity_6_int_to_int() {
        let sig = ForeignSignature {
            params: vec![FfiType::Int; 6],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "sum_6_ints".into(),
            ptr: sum_6_ints as *const (),
            sig,
        };
        let out = call_foreign(
            &sym,
            &[
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
                Value::Int(5),
                Value::Int(6),
            ],
        )
        .unwrap();
        assert!(matches!(out, Value::Int(21)), "got {:?}", out);
    }

    #[test]
    fn arity_6_float_to_float() {
        let sig = ForeignSignature {
            params: vec![FfiType::Float; 6],
            ret: FfiType::Float,
        };
        let sym = ForeignSymbol {
            name: "sum_6_floats".into(),
            ptr: sum_6_floats as *const (),
            sig,
        };
        let out = call_foreign(
            &sym,
            &[
                Value::Float(1.0),
                Value::Float(2.0),
                Value::Float(3.0),
                Value::Float(4.0),
                Value::Float(5.0),
                Value::Float(6.0),
            ],
        )
        .unwrap();
        assert!(
            matches!(out, Value::Float(f) if (f - 21.0).abs() < 1e-9),
            "got {:?}",
            out
        );
    }

    #[test]
    fn arity_7_int_to_int() {
        let sig = ForeignSignature {
            params: vec![FfiType::Int; 7],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "sum_7_ints".into(),
            ptr: sum_7_ints as *const (),
            sig,
        };
        let out = call_foreign(
            &sym,
            &[
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
                Value::Int(5),
                Value::Int(6),
                Value::Int(7),
            ],
        )
        .unwrap();
        assert!(matches!(out, Value::Int(28)), "got {:?}", out);
    }

    #[test]
    fn arity_7_float_to_float() {
        let sig = ForeignSignature {
            params: vec![FfiType::Float; 7],
            ret: FfiType::Float,
        };
        let sym = ForeignSymbol {
            name: "sum_7_floats".into(),
            ptr: sum_7_floats as *const (),
            sig,
        };
        let out = call_foreign(
            &sym,
            &[
                Value::Float(1.0),
                Value::Float(2.0),
                Value::Float(3.0),
                Value::Float(4.0),
                Value::Float(5.0),
                Value::Float(6.0),
                Value::Float(7.0),
            ],
        )
        .unwrap();
        assert!(
            matches!(out, Value::Float(f) if (f - 28.0).abs() < 1e-9),
            "got {:?}",
            out
        );
    }

    #[test]
    fn arity_8_int_to_int() {
        let sig = ForeignSignature {
            params: vec![FfiType::Int; 8],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "sum_8_ints".into(),
            ptr: sum_8_ints as *const (),
            sig,
        };
        let out = call_foreign(
            &sym,
            &[
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
                Value::Int(5),
                Value::Int(6),
                Value::Int(7),
                Value::Int(8),
            ],
        )
        .unwrap();
        assert!(matches!(out, Value::Int(36)), "got {:?}", out);
    }

    #[test]
    fn arity_8_float_to_float() {
        let sig = ForeignSignature {
            params: vec![FfiType::Float; 8],
            ret: FfiType::Float,
        };
        let sym = ForeignSymbol {
            name: "sum_8_floats".into(),
            ptr: sum_8_floats as *const (),
            sig,
        };
        let out = call_foreign(
            &sym,
            &[
                Value::Float(1.0),
                Value::Float(2.0),
                Value::Float(3.0),
                Value::Float(4.0),
                Value::Float(5.0),
                Value::Float(6.0),
                Value::Float(7.0),
                Value::Float(8.0),
            ],
        )
        .unwrap();
        assert!(
            matches!(out, Value::Float(f) if (f - 36.0).abs() < 1e-9),
            "got {:?}",
            out
        );
    }

    // ============================================================
    // RES-317: struct bridging — small structs ≤ 8 bytes by value.
    // ============================================================

    extern "C" fn make_unit_int(v: i64) -> u64 {
        // Build a one-field struct equivalent: low 8 bytes = v.
        v as u64
    }

    extern "C" fn double_unit_int(packed: u64) -> u64 {
        let v = packed as i64;
        (v * 2) as u64
    }

    extern "C" fn read_unit_int(packed: u64) -> i64 {
        packed as i64
    }

    fn one_int_struct_ty() -> FfiType {
        FfiType::Struct {
            name: "OneInt".to_string(),
            fields: vec![("v".to_string(), FfiType::Int)],
        }
    }

    #[test]
    fn struct_pack_unpack_round_trip() {
        // Layout sanity: (Int) → 8 bytes, align 8, offset 0.
        let ty = one_int_struct_ty();
        let layout = struct_layout(match &ty {
            FfiType::Struct { fields, .. } => fields,
            _ => unreachable!(),
        });
        assert_eq!(layout.total, 8);
        assert_eq!(layout.align, 8);
        assert_eq!(layout.offsets, vec![0]);

        let v = Value::Struct {
            name: "OneInt".to_string(),
            fields: vec![("v".to_string(), Value::Int(42))],
        };
        let packed = pack_struct_to_u64(&v, &ty).unwrap();
        assert_eq!(packed, 42_u64);
        let back = unpack_struct_from_u64(packed, &ty).unwrap();
        match back {
            Value::Struct { name, fields } => {
                assert_eq!(name, "OneInt");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].0, "v");
                assert!(matches!(fields[0].1, Value::Int(42)));
            }
            other => panic!("expected Value::Struct, got {:?}", other),
        }
    }

    #[test]
    fn call_foreign_int_to_struct_factory() {
        let sig = ForeignSignature {
            params: vec![FfiType::Int],
            ret: one_int_struct_ty(),
        };
        let sym = ForeignSymbol {
            name: "make_unit_int".to_string(),
            ptr: make_unit_int as *const (),
            sig,
        };
        let out = call_foreign(&sym, &[Value::Int(7)]).unwrap();
        match out {
            Value::Struct { name, fields } => {
                assert_eq!(name, "OneInt");
                assert!(matches!(fields[0].1, Value::Int(7)));
            }
            other => panic!("expected struct, got {:?}", other),
        }
    }

    #[test]
    fn call_foreign_struct_to_struct_round_trip() {
        let sig = ForeignSignature {
            params: vec![one_int_struct_ty()],
            ret: one_int_struct_ty(),
        };
        let sym = ForeignSymbol {
            name: "double_unit_int".to_string(),
            ptr: double_unit_int as *const (),
            sig,
        };
        let arg = Value::Struct {
            name: "OneInt".to_string(),
            fields: vec![("v".to_string(), Value::Int(21))],
        };
        let out = call_foreign(&sym, &[arg]).unwrap();
        match out {
            Value::Struct { fields, .. } => {
                assert!(matches!(fields[0].1, Value::Int(42)));
            }
            other => panic!("expected struct, got {:?}", other),
        }
    }

    #[test]
    fn call_foreign_struct_to_int_readback() {
        let sig = ForeignSignature {
            params: vec![one_int_struct_ty()],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "read_unit_int".to_string(),
            ptr: read_unit_int as *const (),
            sig,
        };
        let arg = Value::Struct {
            name: "OneInt".to_string(),
            fields: vec![("v".to_string(), Value::Int(99))],
        };
        let out = call_foreign(&sym, &[arg]).unwrap();
        assert!(matches!(out, Value::Int(99)));
    }

    #[test]
    fn struct_too_large_errors_cleanly() {
        // Three Int fields = 24 bytes — well over the 8-byte limit.
        let big = FfiType::Struct {
            name: "Big".to_string(),
            fields: vec![
                ("a".to_string(), FfiType::Int),
                ("b".to_string(), FfiType::Int),
                ("c".to_string(), FfiType::Int),
            ],
        };
        let sig = ForeignSignature {
            params: vec![big.clone()],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "bogus_big".to_string(),
            ptr: read_unit_int as *const (), // pointer is unused — must short-circuit.
            sig,
        };
        let arg = Value::Struct {
            name: "Big".to_string(),
            fields: vec![
                ("a".to_string(), Value::Int(1)),
                ("b".to_string(), Value::Int(2)),
                ("c".to_string(), Value::Int(3)),
            ],
        };
        let err = call_foreign(&sym, &[arg]).expect_err("must reject too-large struct");
        assert!(
            err.contains("too large")
                || err.to_lowercase().contains("too large")
                || err.contains("Phase 1"),
            "got {}",
            err
        );
    }

    #[test]
    fn struct_field_mismatch_errors_cleanly() {
        // Pass a struct named "Wrong" where the signature expects "OneInt".
        let sig = ForeignSignature {
            params: vec![one_int_struct_ty()],
            ret: FfiType::Int,
        };
        let sym = ForeignSymbol {
            name: "read_unit_int".to_string(),
            ptr: read_unit_int as *const (),
            sig,
        };
        let arg = Value::Struct {
            name: "Wrong".to_string(),
            fields: vec![("v".to_string(), Value::Int(1))],
        };
        let err = call_foreign(&sym, &[arg]).expect_err("must reject mismatched name");
        assert!(err.contains("type mismatch"), "got {}", err);
    }
}
