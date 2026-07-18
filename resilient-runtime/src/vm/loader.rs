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
use super::{FunctionDef, Instr, Value, Vm, VmError};

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
    /// RES-4077 (D-E1 fn-support): [`load_and_run_with_functions`]'s
    /// blob declares more functions than the loader's fixed-capacity
    /// `out_func_meta` buffer can hold. Retry with a larger
    /// `FUNC_META_N`.
    TooManyFuncs,
    /// RES-4077 (D-E1 fn-support): the combined size of every
    /// function body in [`load_and_run_with_functions`]'s blob
    /// exceeds the loader's fixed-capacity `out_func_code` buffer.
    /// Retry with a larger `FUNC_CODE_N`.
    TooManyFuncInstrs,
    /// The blob decoded cleanly but the VM hit a typed runtime error
    /// while executing it (stack/locals overflow, divide by zero,
    /// bad jump target, operand type mismatch, ...). See [`VmError`].
    VmError(VmError),
}

impl From<DecodeError> for LoaderError {
    fn from(e: DecodeError) -> Self {
        match e {
            DecodeError::TooManyInstrs => LoaderError::TooManyInstrs,
            DecodeError::TooManyFuncs => LoaderError::TooManyFuncs,
            DecodeError::TooManyFuncInstrs => LoaderError::TooManyFuncInstrs,
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

/// RES-4077 (D-E1 fn-support): the function-table counterpart of
/// [`load_and_run`]. Decodes a `.rzbc` blob produced by
/// [`serde::encode_program`] — main chunk plus a function table —
/// into fixed-capacity buffers, then runs it on a fresh [`Vm`] with
/// room for `CALLS` simultaneous call frames.
///
/// `MAIN_N` bounds the main chunk's instruction count, `FUNC_META_N`
/// bounds the number of functions, and `FUNC_CODE_N` bounds the
/// combined instruction count across every function body — all
/// three are `const` generics so the loader stays zero-heap, same
/// as [`load_and_run`].
///
/// ```
/// use resilient_runtime::vm::Instr;
/// use resilient_runtime::vm::serde::{encode_program, EncodeFunctionDef};
/// use resilient_runtime::vm::loader::load_and_run_with_functions;
///
/// let square = [Instr::LoadLocal(0), Instr::LoadLocal(0), Instr::Mul, Instr::Return];
/// let main = [
///     Instr::PushConst(resilient_runtime::vm::Value::Int(9)),
///     Instr::Call(0),
///     Instr::Return,
/// ];
/// let functions = [EncodeFunctionDef { code: &square, arity: 1, local_count: 1, postcheck: None }];
///
/// let mut buf = [0u8; 128];
/// let len = encode_program(&main, &functions, &mut buf).unwrap();
///
/// let result = load_and_run_with_functions::<8, 4, 16, 8, 4, 2>(&buf[..len]);
/// assert_eq!(result, Ok(resilient_runtime::vm::Value::Int(81)));
/// ```
pub fn load_and_run_with_functions<
    const MAIN_N: usize,
    const FUNC_META_N: usize,
    const FUNC_CODE_N: usize,
    const STACK: usize,
    const LOCALS: usize,
    const CALLS: usize,
>(
    blob: &[u8],
) -> Result<Value, LoaderError> {
    let mut main_instrs = [Instr::Return; MAIN_N];
    let mut func_meta = [serde::DecodedFunctionMeta {
        offset: 0,
        len: 0,
        arity: 0,
        local_count: 0,
        postcheck: None,
    }; FUNC_META_N];
    let mut func_code = [Instr::Return; FUNC_CODE_N];

    let counts = serde::decode_program(blob, &mut main_instrs, &mut func_meta, &mut func_code)?;

    let mut functions_buf = [FunctionDef {
        code: &[],
        arity: 0,
        local_count: 0,
        postcheck: None,
    }; FUNC_META_N];
    for (slot, meta) in functions_buf
        .iter_mut()
        .zip(func_meta.iter())
        .take(counts.func_count)
    {
        *slot = FunctionDef {
            code: &func_code[meta.offset as usize..(meta.offset + meta.len) as usize],
            arity: meta.arity,
            local_count: meta.local_count,
            postcheck: meta.postcheck,
        };
    }

    let mut vm = Vm::<STACK, LOCALS, CALLS>::new();
    let result = vm.run_with_functions(
        &functions_buf[..counts.func_count],
        &main_instrs[..counts.main_len],
    )?;
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

    // ---------- RES-4077 (D-E1 fn-support) ----------

    #[test]
    fn load_and_run_with_functions_calls_a_function() {
        let square = [
            Instr::LoadLocal(0),
            Instr::LoadLocal(0),
            Instr::Mul,
            Instr::Return,
        ];
        let main = [
            Instr::PushConst(Value::Int(9)),
            Instr::Call(0),
            Instr::Return,
        ];
        let functions = [serde::EncodeFunctionDef {
            code: &square,
            arity: 1,
            local_count: 1,
            postcheck: None,
        }];
        let mut buf = [0u8; 128];
        let len = serde::encode_program(&main, &functions, &mut buf).unwrap();

        let result = load_and_run_with_functions::<8, 4, 16, 8, 4, 2>(&buf[..len]);
        assert_eq!(result, Ok(Value::Int(81)));
    }

    #[test]
    fn load_and_run_with_functions_no_functions_still_runs_main() {
        let main = [
            Instr::PushConst(Value::Int(2)),
            Instr::PushConst(Value::Int(3)),
            Instr::Add,
            Instr::Return,
        ];
        let mut buf = [0u8; 64];
        let len = serde::encode_program(&main, &[], &mut buf).unwrap();

        let result = load_and_run_with_functions::<8, 0, 0, 8, 0, 1>(&buf[..len]);
        assert_eq!(result, Ok(Value::Int(5)));
    }

    #[test]
    fn load_and_run_with_functions_recursion_depth_exhaustion_is_typed_error() {
        let countdown = [
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(0)),
            Instr::Gt,
            Instr::JumpIfFalse(9),
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(1)),
            Instr::Sub,
            Instr::Call(0),
            Instr::Return,
            Instr::LoadLocal(0),
            Instr::Return,
        ];
        let main = [
            Instr::PushConst(Value::Int(50)),
            Instr::Call(0),
            Instr::Return,
        ];
        let functions = [serde::EncodeFunctionDef {
            code: &countdown,
            arity: 1,
            local_count: 1,
            postcheck: None,
        }];
        let mut buf = [0u8; 256];
        let len = serde::encode_program(&main, &functions, &mut buf).unwrap();

        // CALLS == 3 caps recursion well short of depth 50.
        let result = load_and_run_with_functions::<8, 4, 32, 8, 4, 3>(&buf[..len]);
        assert_eq!(
            result,
            Err(LoaderError::VmError(VmError::CallStackOverflow))
        );
    }

    #[test]
    fn load_and_run_with_functions_bad_magic_is_decode_failed_not_a_panic() {
        let mut buf = [0u8; serde::HEADER_LEN];
        buf[..4].copy_from_slice(b"NOPE");
        let result = load_and_run_with_functions::<4, 4, 4, 8, 0, 1>(&buf);
        assert_eq!(
            result,
            Err(LoaderError::DecodeFailed(DecodeError::BadMagic))
        );
    }

    #[test]
    fn load_and_run_with_functions_too_many_funcs_is_typed_error_not_a_panic() {
        let f1 = [Instr::Return];
        let f2 = [Instr::Return];
        let functions = [
            serde::EncodeFunctionDef {
                code: &f1,
                arity: 0,
                local_count: 0,
                postcheck: None,
            },
            serde::EncodeFunctionDef {
                code: &f2,
                arity: 0,
                local_count: 0,
                postcheck: None,
            },
        ];
        let mut buf = [0u8; 128];
        let len = serde::encode_program(&[], &functions, &mut buf).unwrap();

        // FUNC_META_N == 1 can't hold 2 function-table entries.
        let result = load_and_run_with_functions::<4, 1, 8, 8, 0, 1>(&buf[..len]);
        assert_eq!(result, Err(LoaderError::TooManyFuncs));
    }
}
