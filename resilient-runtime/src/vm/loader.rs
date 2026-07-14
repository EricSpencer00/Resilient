//! RES-3987 (D-E1): the on-device `.rzbc` loader — the glue between
//! [`super::serde::decode`] and [`super::Vm`] that a thin `no_std`
//! binary needs to go from "embedded byte blob" to "executed
//! result" in one call.
//!
//! `docs/EMBEDDED_PIPELINE.md` section 3.3 sketches a loader
//! template that (1) embeds a `.rzbc` blob as a `static` byte array
//! via `include_bytes!`, (2) decodes it into a fixed-capacity
//! instruction buffer, (3) constructs a [`super::Vm`] sized from the
//! target's stack/locals budget, and (4) runs it to completion or
//! error. [`load_and_run`] is exactly that sequence, factored out so
//! it is host-testable (this module's tests round-trip a real
//! encoded program through the real decoder and VM) and reusable by
//! both a host harness and an on-device binary — see
//! `resilient-runtime-loader-demo/` for the latter.
//!
//! # No heap, no panics
//!
//! The decode buffer is a fixed-capacity `[Instr; N]` array sized by
//! a `const` generic, matching the `Vm<STACK, LOCALS>` idiom
//! `super` already uses. Every fallible step — decode failure, an
//! instruction count that overflows `N`, or a VM runtime error —
//! returns a typed [`LoaderError`] instead of panicking.

use super::serde::{self, DecodeError};
use super::{Instr, Value, Vm, VmError};

/// Errors [`load_and_run`] can return. Wraps the two error sources
/// it composes — [`DecodeError`] and [`VmError`] — plus a
/// loader-level `TooManyInstrs` pulled out of `DecodeFailed`: it is
/// the one failure a caller can resolve simply by re-instantiating
/// `load_and_run` with a larger `N`, so it gets its own variant
/// instead of being buried inside an opaque `DecodeFailed(..)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderError {
    /// The blob failed to decode for a reason other than capacity
    /// (bad magic, unsupported version, truncated input, a bad tag
    /// or operand byte). See [`DecodeError`] for the specific cause.
    DecodeFailed(DecodeError),
    /// The blob's header declares more instructions than the
    /// loader's fixed-capacity buffer (`N`) can hold. Retry with a
    /// larger `N`.
    TooManyInstrs,
    /// The blob decoded cleanly but the VM hit a typed runtime error
    /// while executing it (stack/locals overflow, divide by zero,
    /// bad jump target, operand type mismatch, ...). See [`VmError`].
    VmError(VmError),
}

impl From<DecodeError> for LoaderError {
    fn from(e: DecodeError) -> Self {
        match e {
            DecodeError::TooManyInstrs => LoaderError::TooManyInstrs,
            other => LoaderError::DecodeFailed(other),
        }
    }
}

impl From<VmError> for LoaderError {
    fn from(e: VmError) -> Self {
        LoaderError::VmError(e)
    }
}

/// Decode `blob` (a `.rzbc` byte stream — see [`super::serde`]) into
/// a fixed-capacity buffer of at most `N` instructions, then run it
/// to completion on a fresh [`Vm`] with `STACK` operand-stack slots
/// and `LOCALS` local-variable slots.
///
/// This is the full on-device pipeline in one call: a loader binary
/// only needs to `include_bytes!` its `.rzbc` blob and pick `N`,
/// `STACK`, and `LOCALS` sized for its program (typically fixed
/// `const`s chosen at build time — see
/// `resilient-runtime-loader-demo/src/main.rs`).
///
/// Never panics: an oversized blob, a malformed blob, or a program
/// that hits a VM error all return `Err(LoaderError)` rather than
/// aborting.
///
/// ```
/// use resilient_runtime::vm::{Instr, Value};
/// use resilient_runtime::vm::serde::encode;
/// use resilient_runtime::vm::loader::load_and_run;
///
/// let program = [
///     Instr::PushConst(Value::Int(2)),
///     Instr::PushConst(Value::Int(3)),
///     Instr::Add,
///     Instr::Return,
/// ];
/// let mut buf = [0u8; 64];
/// let len = encode(&program, &mut buf).unwrap();
///
/// let result = load_and_run::<8, 8, 0>(&buf[..len]);
/// assert_eq!(result, Ok(Value::Int(5)));
/// ```
pub fn load_and_run<const N: usize, const STACK: usize, const LOCALS: usize>(
    blob: &[u8],
) -> Result<Value, LoaderError> {
    let mut instrs = [Instr::Return; N];
    let count = serde::decode(blob, &mut instrs)?;
    let mut vm = Vm::<STACK, LOCALS>::new();
    let result = vm.run(&instrs[..count])?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact fixture `resilient-runtime-loader-demo` embeds via
    /// `include_bytes!` — committed so both the host test and the
    /// on-device binary run the identical bytes. Encodes
    /// `(2 + 3) * 4 + 1 == 21`.
    const ARITHMETIC_DEMO_RZBC: &[u8] = include_bytes!("../../fixtures/arithmetic_demo.rzbc");

    #[test]
    fn load_and_run_committed_fixture_round_trips_through_real_decoder_and_vm() {
        let result = load_and_run::<16, 8, 0>(ARITHMETIC_DEMO_RZBC);
        assert_eq!(result, Ok(Value::Int(21)));
    }

    #[test]
    fn load_and_run_inline_encoded_program() {
        let program = [
            Instr::PushConst(Value::Int(2)),
            Instr::PushConst(Value::Int(3)),
            Instr::Add,
            Instr::Return,
        ];
        let mut buf = [0u8; 64];
        let len = serde::encode(&program, &mut buf).unwrap();

        let result = load_and_run::<8, 8, 0>(&buf[..len]);
        assert_eq!(result, Ok(Value::Int(5)));
    }

    #[test]
    fn load_and_run_program_using_locals() {
        let program = [
            Instr::PushConst(Value::Int(10)),
            Instr::StoreLocal(0),
            Instr::LoadLocal(0),
            Instr::LoadLocal(0),
            Instr::Mul,
            Instr::Return,
        ];
        let mut buf = [0u8; 64];
        let len = serde::encode(&program, &mut buf).unwrap();

        let result = load_and_run::<8, 8, 1>(&buf[..len]);
        assert_eq!(result, Ok(Value::Int(100)));
    }

    #[test]
    fn load_and_run_bad_magic_is_decode_failed_not_a_panic() {
        let mut buf = [0u8; serde::HEADER_LEN];
        buf[..4].copy_from_slice(b"NOPE");
        let result = load_and_run::<4, 8, 0>(&buf);
        assert_eq!(
            result,
            Err(LoaderError::DecodeFailed(DecodeError::BadMagic))
        );
    }

    #[test]
    fn load_and_run_truncated_input_is_decode_failed_not_a_panic() {
        let result = load_and_run::<4, 8, 0>(&[]);
        assert_eq!(
            result,
            Err(LoaderError::DecodeFailed(DecodeError::Truncated))
        );
    }

    #[test]
    fn load_and_run_too_many_instrs_is_typed_error_not_a_panic() {
        let program = [Instr::Return, Instr::Return, Instr::Return];
        let mut buf = [0u8; 64];
        let len = serde::encode(&program, &mut buf).unwrap();

        // N == 2 can't hold 3 decoded instructions.
        let result = load_and_run::<2, 8, 0>(&buf[..len]);
        assert_eq!(result, Err(LoaderError::TooManyInstrs));
    }

    #[test]
    fn load_and_run_vm_error_propagates_as_typed_error_not_a_panic() {
        let program = [
            Instr::PushConst(Value::Int(1)),
            Instr::PushConst(Value::Int(0)),
            Instr::Div,
            Instr::Return,
        ];
        let mut buf = [0u8; 64];
        let len = serde::encode(&program, &mut buf).unwrap();

        let result = load_and_run::<8, 8, 0>(&buf[..len]);
        assert_eq!(result, Err(LoaderError::VmError(VmError::DivideByZero)));
    }

    #[test]
    fn load_and_run_stack_overflow_propagates_as_typed_error_not_a_panic() {
        let mut program = [Instr::PushConst(Value::Int(1)); 6];
        program[5] = Instr::Return;
        let mut buf = [0u8; 128];
        let len = serde::encode(&program, &mut buf).unwrap();

        // STACK == 4 can't hold 5 pushed values.
        let result = load_and_run::<8, 4, 0>(&buf[..len]);
        assert_eq!(result, Err(LoaderError::VmError(VmError::StackOverflow)));
    }
}
