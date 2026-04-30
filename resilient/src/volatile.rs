//! RES-406: volatile MMIO intrinsics.
//!
//! Per the design lock-in spec
//! ([docs/superpowers/specs/2026-04-30-mmio-isr-design.md](../../docs/superpowers/specs/2026-04-30-mmio-isr-design.md)),
//! V1 ships eight fixed-width intrinsics — one read + one write
//! for each of u8, u16, u32, u64. The generic `volatile_read<T>`
//! facade lands once RES-405 (generics) is in.
//!
//! Each intrinsic:
//!
//! * Takes the address as `int` (i64) and reads/writes the bit
//!   pattern at that address.
//! * Is a regular builtin in `BUILTINS` — the `unsafe { … }` gate
//!   is enforced by the typechecker, not the runtime, so the
//!   interpreter / VM / JIT all share the same dispatch path.
//! * Lowers to `core::ptr::read_volatile` / `write_volatile` in
//!   the runtime path. The pointer cast is justified by the
//!   `unsafe { … }` gate at the call site.
//!
//! The intrinsics are tagged "non-modelable" by the verifier —
//! same path FFI takes (per the [TLA+ V2.0 design lock-in's Q4](../../docs/superpowers/specs/2026-04-30-tla-v2-design-lock-in.md#q4-ffi-side-effects--choose-or-contract)).
//! Returns from a volatile read are nondeterministic to Z3
//! (`CHOOSE x \in T : true`); writes are no-ops at the Z3 level.
//!
//! Tests use a small allocated buffer and the address-of trick —
//! we cast a `Vec<u8>` pointer back to `usize`, hand it to the
//! intrinsic, and verify round-trip semantics. No real MMIO
//! addresses are touched.

use crate::{RResult, Value};

/// Type-tag for the eight intrinsic variants. Used by the unified
/// `read` / `write` paths to centralize bounds + alignment checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    U8,
    U16,
    U32,
    U64,
}

impl Width {
    fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::U32 => 4,
            Self::U64 => 8,
        }
    }

    /// Width name for diagnostics. Read by future tooling; not used
    /// directly today but kept for symmetry with `bytes()`.
    #[allow(dead_code)]
    fn name(self) -> &'static str {
        match self {
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
        }
    }
}

/// Common implementation: read `width` bytes from `addr` and return
/// the resulting unsigned value, zero-extended into i64 for the
/// runtime's int slot.
///
/// # Safety
///
/// The caller must ensure `addr` is a valid, aligned pointer to
/// `width` bytes of memory the program is permitted to read. The
/// `unsafe { … }` block at the call site is the language-level
/// authorization for this; the runtime can't verify it without
/// modeling the address space, so we trust the caller.
fn read_at(addr: usize, width: Width) -> u64 {
    unsafe {
        match width {
            Width::U8 => core::ptr::read_volatile(addr as *const u8) as u64,
            Width::U16 => core::ptr::read_volatile(addr as *const u16) as u64,
            Width::U32 => core::ptr::read_volatile(addr as *const u32) as u64,
            Width::U64 => core::ptr::read_volatile(addr as *const u64),
        }
    }
}

/// Common implementation: write the low `width` bytes of `value` to
/// `addr` as a volatile store.
fn write_at(addr: usize, value: u64, width: Width) {
    unsafe {
        match width {
            Width::U8 => core::ptr::write_volatile(addr as *mut u8, value as u8),
            Width::U16 => core::ptr::write_volatile(addr as *mut u16, value as u16),
            Width::U32 => core::ptr::write_volatile(addr as *mut u32, value as u32),
            Width::U64 => core::ptr::write_volatile(addr as *mut u64, value),
        }
    }
}

/// Validate the address argument. Bounds-checks that `addr` fits
/// in `usize` (the address-space pointer width on the host) and is
/// non-negative; a negative `addr` is almost certainly a bug,
/// definitely not a valid MMIO address.
fn coerce_addr(name: &'static str, args: &[Value]) -> Result<usize, String> {
    match args.first() {
        Some(Value::Int(addr)) => {
            if *addr < 0 {
                return Err(format!("{name}: address must be non-negative, got {addr}"));
            }
            usize::try_from(*addr)
                .map_err(|_| format!("{name}: address {addr} does not fit in usize"))
        }
        Some(other) => Err(format!("{name}: expected (int, ...), got ({other}, ...)")),
        None => Err(format!("{name}: expected at least 1 argument, got 0")),
    }
}

/// Generic read-builtin body parameterised by `Width`. The width
/// is captured by the calling builtin function (see the eight
/// monomorphic shims below).
fn builtin_volatile_read(name: &'static str, args: &[Value], width: Width) -> RResult<Value> {
    if args.len() != 1 {
        return Err(format!(
            "{name}: expected 1 argument (address), got {}",
            args.len()
        ));
    }
    let addr = coerce_addr(name, args)?;
    if addr % width.bytes() != 0 {
        return Err(format!(
            "{name}: address {addr:#x} is not aligned to {} bytes",
            width.bytes()
        ));
    }
    Ok(Value::Int(read_at(addr, width) as i64))
}

/// Generic write-builtin body parameterised by `Width`.
fn builtin_volatile_write(name: &'static str, args: &[Value], width: Width) -> RResult<Value> {
    if args.len() != 2 {
        return Err(format!(
            "{name}: expected 2 arguments (address, value), got {}",
            args.len()
        ));
    }
    let addr = coerce_addr(name, args)?;
    if addr % width.bytes() != 0 {
        return Err(format!(
            "{name}: address {addr:#x} is not aligned to {} bytes",
            width.bytes()
        ));
    }
    let value = match &args[1] {
        Value::Int(n) => *n as u64,
        other => {
            return Err(format!("{name}: expected (int, int), got (..., {other})"));
        }
    };
    write_at(addr, value, width);
    Ok(Value::Void)
}

// The eight shipped intrinsics. Each is a thin shim over the
// generic body that pins the width.

pub fn volatile_read_u8(args: &[Value]) -> RResult<Value> {
    builtin_volatile_read("volatile_read_u8", args, Width::U8)
}
pub fn volatile_read_u16(args: &[Value]) -> RResult<Value> {
    builtin_volatile_read("volatile_read_u16", args, Width::U16)
}
pub fn volatile_read_u32(args: &[Value]) -> RResult<Value> {
    builtin_volatile_read("volatile_read_u32", args, Width::U32)
}
pub fn volatile_read_u64(args: &[Value]) -> RResult<Value> {
    builtin_volatile_read("volatile_read_u64", args, Width::U64)
}
pub fn volatile_write_u8(args: &[Value]) -> RResult<Value> {
    builtin_volatile_write("volatile_write_u8", args, Width::U8)
}
pub fn volatile_write_u16(args: &[Value]) -> RResult<Value> {
    builtin_volatile_write("volatile_write_u16", args, Width::U16)
}
pub fn volatile_write_u32(args: &[Value]) -> RResult<Value> {
    builtin_volatile_write("volatile_write_u32", args, Width::U32)
}
pub fn volatile_write_u64(args: &[Value]) -> RResult<Value> {
    builtin_volatile_write("volatile_write_u64", args, Width::U64)
}

/// Names of the eight intrinsics, in (read, write, width) tuples.
/// Used by the typechecker (RES-406's `unsafe { … }` gate) to
/// refuse calls outside an unsafe block.
pub const VOLATILE_INTRINSIC_NAMES: &[&str] = &[
    "volatile_read_u8",
    "volatile_read_u16",
    "volatile_read_u32",
    "volatile_read_u64",
    "volatile_write_u8",
    "volatile_write_u16",
    "volatile_write_u32",
    "volatile_write_u64",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn buf_addr(buf: &[u8]) -> i64 {
        buf.as_ptr() as i64
    }
    fn buf_addr_mut(buf: &mut [u8]) -> i64 {
        buf.as_mut_ptr() as i64
    }

    #[test]
    fn round_trip_u8() {
        let mut buf = vec![0u8; 1];
        let addr = buf_addr_mut(&mut buf);
        let _ = volatile_write_u8(&[Value::Int(addr), Value::Int(0xAB)]).unwrap();
        let v = volatile_read_u8(&[Value::Int(addr)]).unwrap();
        match v {
            Value::Int(n) => assert_eq!(n, 0xAB),
            other => panic!("expected Int(0xAB), got {:?}", other),
        }
    }

    #[test]
    fn round_trip_u16_aligned() {
        let mut buf = vec![0u8; 2];
        let addr = buf_addr_mut(&mut buf);
        let _ = volatile_write_u16(&[Value::Int(addr), Value::Int(0xCAFE)]).unwrap();
        let v = volatile_read_u16(&[Value::Int(addr)]).unwrap();
        match v {
            Value::Int(n) => assert_eq!(n, 0xCAFE),
            other => panic!("expected Int(0xCAFE), got {:?}", other),
        }
    }

    #[test]
    fn round_trip_u32_aligned() {
        // Force alignment by using a fixed-size array.
        let buf: [u32; 1] = [0];
        let addr = buf_addr(unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, 4) });
        let _ = volatile_write_u32(&[Value::Int(addr), Value::Int(0xDEAD_BEEF_i64)]).unwrap();
        let v = volatile_read_u32(&[Value::Int(addr)]).unwrap();
        match v {
            Value::Int(n) => assert_eq!(n as u32, 0xDEADBEEFu32),
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn round_trip_u64_aligned() {
        let buf: [u64; 1] = [0];
        let addr = buf_addr(unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, 8) });
        let _ = volatile_write_u64(&[Value::Int(addr), Value::Int(0x0102030405060708)]).unwrap();
        let v = volatile_read_u64(&[Value::Int(addr)]).unwrap();
        match v {
            Value::Int(n) => assert_eq!(n, 0x0102030405060708),
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn unaligned_read_errors_for_u32() {
        // Address 1 inside a 4-byte buffer is not 4-aligned.
        let buf = vec![0u8; 8];
        let addr = buf_addr(&buf) + 1;
        let err = volatile_read_u32(&[Value::Int(addr)]).unwrap_err();
        assert!(err.contains("not aligned"), "got: {}", err);
    }

    #[test]
    fn negative_address_errors() {
        let err = volatile_read_u8(&[Value::Int(-1)]).unwrap_err();
        assert!(err.contains("non-negative"), "got: {}", err);
    }

    #[test]
    fn wrong_arity_errors() {
        let err = volatile_read_u8(&[]).unwrap_err();
        assert!(err.contains("expected 1 argument"), "got: {}", err);
        let err = volatile_write_u8(&[Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected 2 arguments"), "got: {}", err);
    }

    #[test]
    fn intrinsic_name_list_matches_eight_known() {
        assert_eq!(VOLATILE_INTRINSIC_NAMES.len(), 8);
        assert!(VOLATILE_INTRINSIC_NAMES.contains(&"volatile_read_u8"));
        assert!(VOLATILE_INTRINSIC_NAMES.contains(&"volatile_write_u64"));
    }
}
