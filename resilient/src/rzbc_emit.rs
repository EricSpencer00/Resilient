//! RES-3987 (D-E1): compiler-side emitter for the `.rzbc` wire format
//! that [`resilient_runtime::vm::serde`] decodes on the embedded
//! side. This is the third PR in the D-E1 sequence
//! (`docs/EMBEDDED_PIPELINE.md` section 5, items 1-3): #4031 shipped
//! the no_std `Instr`/`Vm`, #4034 shipped the `.rzbc` encoder/decoder
//! *inside* `resilient-runtime`, and this module is the missing other
//! half — mapping the host compiler's [`Op`] stream (`bytecode.rs`)
//! onto [`Instr`] so `rz build --target <TRIPLE>` (see `lib.rs`) has
//! something to feed [`resilient_runtime::vm::serde::encode`].
//!
//! # Scope: a deliberately narrow bridge, not a full port
//!
//! `docs/EMBEDDED_PIPELINE.md` section 1 audits `Op`'s 54 variants
//! into "no_std-clean" (arithmetic/comparison/control-flow/locals)
//! vs. "alloc-required" (anything touching a heap-bearing `Value`).
//! [`Instr`] has dispatch arms for the no_std-clean subset plus
//! (RES-4077, D-E1 fn-support) plain top-level function calls —
//! `Op::Call`/`Op::ReturnFromCall` translate to `Instr::Call`/
//! `Instr::Return` against the embedded VM's bounded call-frame
//! stack (`resilient_runtime::vm::Vm::run_with_functions`). Bitwise
//! ops are still not ported to `Instr`. This module enforces exactly
//! that subset at compile time:
//!
//! - Every [`Program::functions`] entry must be a plain top-level
//!   `fn`: no captured upvalues (closures), no declared `fails`
//!   variants (checked-failure catch dispatch has no embedded
//!   equivalent — the translated `Call`/`Return` pair does not walk
//!   a try-handler table). Each of these produces a typed
//!   [`EmitError`] naming the function.
//! - (RES-4083, D-E1 tail) a synthesized postcondition-check
//!   function (`ensures`/`recovers_to`) is supported: the embedded
//!   `Instr::Return` now invokes the postcheck function — itself
//!   just another `program.functions` entry — as an isolated nested
//!   call, mirroring the host VM's `run_postcheck`. See
//!   `resilient_runtime::vm::FunctionDef::postcheck`.
//! - Every function/`main` [`Op`] must be one this module knows how
//!   to translate 1:1 into an [`Instr`] (see [`translate_chunk`]).
//!   Anything else — `IncLocal`, arrays, structs, enums,
//!   closures, try/catch, FFI, builtins, bitwise ops,
//!   `CallClosure`/`CallMethod`/`CallForeign`/
//!   `CallBuiltin`, ... — is a typed [`EmitError`] naming the exact
//!   opcode, never a silently malformed blob.
//! - Every `Op::Const` constant must be `Value::Int`/`Bool`/`Float`
//!   — `Instr::PushConst` carries the value inline (no separate
//!   constant pool in the `.rzbc` format; see
//!   `resilient_runtime::vm::serde`'s module docs for the wire
//!   layout this mirrors), so a `String`/`Array`/... constant has no
//!   representation to translate into. This also means every
//!   function parameter and return value is implicitly scalar —
//!   there is no representation for a non-scalar argument to have
//!   arrived on the stack in the first place.
//!
//! This 1:1, index-preserving translation is only sound because the
//! host compiler's [`peephole`](crate::peephole) pass already
//! recomputes every `Jump`/`JumpIfFalse`/`JumpIfTrue` relative
//! offset against the *final* (already-optimized) `Chunk::code`
//! before `compiler::compile` returns it — see [`jump_target`] for
//! how those relative offsets become the `.rzbc` format's absolute
//! `u32` targets.
//!
//! # Known gap: empty-stack `Return`
//!
//! The host VM's `Op::Return` tolerates an empty operand stack
//! (returns `Value::Void`); the embedded [`resilient_runtime::vm::Vm`]'s
//! `Instr::Return` pops the stack and surfaces `VmError::StackUnderflow`
//! on empty. A program whose last top-level statement is *not* a bare
//! expression (e.g. it ends with `let`/an assignment) type-checks and
//! builds cleanly under this module's subset check, but diverges at
//! *runtime* between the two backends. Closing this gap needs either
//! a `Void` variant on the embedded `Value` or a static "does this
//! chunk always leave exactly one value for `Return`" analysis —
//! both out of scope for this bridge PR. Documented, not silently
//! swept under the rug; a real embedded program that wants a return
//! value should end with a bare expression (mirrors the function-body
//! implicit-return convention `compiler.rs` already uses).

use crate::Value as HostValue;
use crate::bytecode::{Chunk, Op, Program};
use resilient_runtime::vm::serde as rzbc_serde;
use resilient_runtime::vm::serde::EncodeFunctionDef;
use resilient_runtime::vm::{Instr, Value as RtValue};

/// A construct in the compiled [`Program`] that has no representation
/// in the embedded no_std [`Instr`] subset. `target` is the triple
/// passed to `rz build --target`; `reason` names the exact opcode or
/// constant type and points at why it's out of scope, so the CLI can
/// surface a diagnostic like:
///
/// ```text
/// error: not supported for embedded target `thumbv7em-none-eabihf`: opcode `CallBuiltin { .. }` is not supported — ...
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct EmitError {
    pub target: String,
    pub reason: String,
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "not supported for embedded target `{}`: {}",
            self.target, self.reason
        )
    }
}

impl std::error::Error for EmitError {}

fn unsupported(target: &str, reason: String) -> EmitError {
    EmitError {
        target: target.to_string(),
        reason,
    }
}

/// Every [`Instr`] variant's wire-encoded width, worst case: 1 tag
/// byte + up to a 1-byte value-tag + an 8-byte `i64`/`f64` payload
/// (`Instr::PushConst(Value::Int(_) | Value::Float(_))`). Used to
/// pre-size the encode buffer generously rather than compute an
/// exact byte count up front — `resilient_runtime::vm::serde::encode`
/// itself is the source of truth for the actual layout and always
/// bounds-checks every write.
const MAX_INSTR_WIRE_WIDTH: usize = 10;

/// Compile `program` (the output of [`crate::compiler::compile`]) to
/// a `.rzbc` byte blob for `target`, or a typed [`EmitError`] naming
/// the first unsupported construct encountered. Never emits a
/// partial/malformed blob on error — the whole translation happens
/// before any bytes are written.
///
/// If `program.functions` is non-empty, emits the
/// [`rzbc_serde::encode_program`] function-table format (RES-4077,
/// D-E1 fn-support); otherwise emits the flat [`rzbc_serde::encode`]
/// format unchanged (byte-for-byte identical to before fn-support
/// landed, so no existing `.rzbc` consumer regresses).
pub fn compile_to_rzbc(program: &Program, target: &str) -> Result<Vec<u8>, EmitError> {
    let main_instrs = translate_chunk(&program.main, target)?;

    if program.functions.is_empty() {
        let cap = rzbc_serde::HEADER_LEN + main_instrs.len() * MAX_INSTR_WIRE_WIDTH;
        let mut buf = vec![0u8; cap];
        let len = rzbc_serde::encode(&main_instrs, &mut buf).map_err(|e| {
            unsupported(
                target,
                format!(
                    "internal error serializing the `.rzbc` blob ({:?}) — this is a bug in \
                     rzbc_emit's buffer sizing, not a property of the source program",
                    e
                ),
            )
        })?;
        buf.truncate(len);
        return Ok(buf);
    }

    let mut func_instrs: Vec<Vec<Instr>> = Vec::with_capacity(program.functions.len());
    for func in &program.functions {
        if !func.upvalue_source_slots.is_empty() {
            return Err(unsupported(
                target,
                format!(
                    "function `{}` captures {} upvalue(s) (a closure) — the embedded no_std VM's \
                     call-frame stack has no upvalue slab, only plain top-level function calls",
                    func.name,
                    func.upvalue_source_slots.len()
                ),
            ));
        }
        if !func.fails.is_empty() {
            return Err(unsupported(
                target,
                format!(
                    "function `{}` declares `fails` variant(s) ({}) — checked-failure catch \
                     dispatch has no embedded equivalent; the translated `Call`/`Return` pair \
                     does not walk a try-handler table",
                    func.name,
                    func.fails.join(", ")
                ),
            ));
        }
        // RES-4083 (D-E1 tail): a function's synthesized postcheck
        // (`ensures`/`recovers_to` — see `compiler::build_postcheck_function`)
        // is itself a plain top-level `fn` entry in `program.functions`
        // with no upvalues/`fails`/postcheck of its own, so it passes
        // this same loop's checks unmodified and translates like any
        // other function. `func.postcheck` is a `program.functions`
        // index, and this loop emits exactly one `EncodeFunctionDef`
        // per `program.functions` entry in order, so the index carries
        // over unchanged into the embedded function table —
        // `resilient_runtime::vm::Vm::execute`'s `Instr::Return` arm
        // invokes it as an isolated nested call, mirroring the host
        // VM's `run_postcheck`.
        func_instrs.push(translate_chunk(&func.chunk, target)?);
    }

    let functions: Vec<EncodeFunctionDef<'_>> = program
        .functions
        .iter()
        .zip(func_instrs.iter())
        .map(|(func, instrs)| EncodeFunctionDef {
            code: instrs.as_slice(),
            arity: func.arity,
            local_count: func.local_count,
            postcheck: func.postcheck,
        })
        .collect();

    let func_instr_total: usize = func_instrs.iter().map(Vec::len).sum();
    let cap = rzbc_serde::HEADER_LEN
        + (main_instrs.len() + func_instr_total + functions.len()) * MAX_INSTR_WIRE_WIDTH
        + functions.len() * 8;
    let mut buf = vec![0u8; cap];
    let len = rzbc_serde::encode_program(&main_instrs, &functions, &mut buf).map_err(|e| {
        unsupported(
            target,
            format!(
                "internal error serializing the `.rzbc` blob ({:?}) — this is a bug in \
                 rzbc_emit's buffer sizing, not a property of the source program",
                e
            ),
        )
    })?;
    buf.truncate(len);
    Ok(buf)
}

/// Translate every [`Op`] in `chunk.code` to the matching [`Instr`],
/// 1:1 by index (`out[i]` is the translation of `chunk.code[i]`) —
/// see the module docs for why this index-preserving property is
/// exactly what makes [`jump_target`]'s offset-to-absolute-index math
/// sound.
fn translate_chunk(chunk: &Chunk, target: &str) -> Result<Vec<Instr>, EmitError> {
    let mut out = Vec::with_capacity(chunk.code.len());
    for (i, op) in chunk.code.iter().enumerate() {
        let instr = match *op {
            Op::Const(idx) => Instr::PushConst(translate_const(chunk, idx, target)?),
            Op::Add => Instr::Add,
            Op::Sub => Instr::Sub,
            Op::Mul => Instr::Mul,
            Op::Div => Instr::Div,
            Op::Mod => Instr::Rem,
            Op::Neg => Instr::Neg,
            Op::LoadLocal(idx) => Instr::LoadLocal(idx),
            Op::StoreLocal(idx) => Instr::StoreLocal(idx),
            Op::Eq => Instr::Eq,
            Op::Neq => Instr::Neq,
            Op::Lt => Instr::Lt,
            Op::Le => Instr::Le,
            Op::Gt => Instr::Gt,
            Op::Ge => Instr::Ge,
            Op::Not => Instr::Not,
            Op::Return => Instr::Return,
            // RES-4077 (D-E1 fn-support): function bodies end with
            // `ReturnFromCall`, not `Return` (see `compiler.rs`) —
            // both pop TOS and hand it back to the caller, which is
            // exactly what the embedded `Instr::Return` does for
            // both the entry chunk and a callee chunk (see
            // `resilient_runtime::vm::Vm::run_with_functions`).
            Op::ReturnFromCall => Instr::Return,
            Op::Call(idx) => Instr::Call(idx),
            // RES-4075 (D-E1 fn-support tail): the peephole pass
            // rewrites a self-recursive `Call(i); ReturnFromCall`
            // pair into `TailCall(i)` (see `compiler.rs`), so any
            // tail-recursive fn reaches this emitter as `TailCall`.
            Op::TailCall(idx) => Instr::TailCall(idx),
            // RES-4075: the compiler emits `Pop` after every
            // non-final expression statement (e.g. `f(1);`), so
            // multi-statement programs need it.
            Op::Pop => Instr::Pop,
            Op::Jump(offset) => Instr::Jump(jump_target(i, offset, target)?),
            Op::JumpIfFalse(offset) => Instr::JumpIfFalse(jump_target(i, offset, target)?),
            Op::JumpIfTrue(offset) => Instr::JumpIfTrue(jump_target(i, offset, target)?),
            // Everything else is (b)-class or otherwise absent from
            // `Instr` (bitwise ops, `IncLocal`, try/catch,
            // FFI, builtins, arrays/structs/enums/closures/tuples,
            // globals). `{:?}` on `Op` names the exact variant so the
            // diagnostic is actionable without a giant match arm per
            // variant name.
            ref other => {
                return Err(unsupported(
                    target,
                    format!(
                        "opcode `{:?}` is not supported — the embedded no_std VM subset covers \
                         only Int/Bool/Float arithmetic, comparisons, control flow, and locals \
                         (see docs/EMBEDDED_PIPELINE.md section 1's opcode audit)",
                        other
                    ),
                ));
            }
        };
        out.push(instr);
    }
    Ok(out)
}

/// Resolve `chunk.constants[idx]` to the [`RtValue`] `Instr::PushConst`
/// carries inline. Only `Int`/`Bool`/`Float` are representable — the
/// `.rzbc` format has no constant pool of its own (unlike the design
/// doc's original section-3.2 sketch), so every constant must be
/// inlinable directly onto the instruction, which rules out anything
/// heap-bearing.
fn translate_const(chunk: &Chunk, idx: u16, target: &str) -> Result<RtValue, EmitError> {
    match chunk.constants.get(idx as usize) {
        Some(HostValue::Int(v)) => Ok(RtValue::Int(*v)),
        Some(HostValue::Bool(b)) => Ok(RtValue::Bool(*b)),
        Some(HostValue::Float(f)) => Ok(RtValue::Float(*f)),
        Some(other) => Err(unsupported(
            target,
            format!(
                "constant `{:?}` is not supported — only Int/Bool/Float constants are \
                 representable in the embedded no_std VM (no String/Array/Struct/... heap types)",
                other
            ),
        )),
        None => Err(unsupported(
            target,
            format!(
                "internal error: constant pool index {} is out of bounds ({} entries) — this is \
                 a bug in the host compiler, not a property of the source program",
                idx,
                chunk.constants.len()
            ),
        )),
    }
}

/// Convert an `Op::Jump*`-style relative offset (relative to the PC
/// *after* the jump, at index `i + 1`) into the `.rzbc` format's
/// absolute `u32` instruction index. Sound only because
/// [`translate_chunk`] emits exactly one `Instr` per `Op`, so index
/// `i` in `chunk.code` and index `i` in the translated `Instr` stream
/// always refer to the same instruction.
fn jump_target(i: usize, offset: i16, target: &str) -> Result<u32, EmitError> {
    let pc_after = i as i64 + 1;
    let dest = pc_after + offset as i64;
    u32::try_from(dest).map_err(|_| {
        unsupported(
            target,
            format!(
                "jump target {} (from instruction {} with offset {}) does not fit the `.rzbc` \
                 format's u32 absolute-target encoding",
                dest, i, offset
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Function;

    fn chunk_from(code: Vec<Op>, constants: Vec<HostValue>) -> Chunk {
        let mut chunk = Chunk::new();
        chunk.code = code;
        chunk.constants = constants;
        chunk
    }

    fn program_from(main: Chunk) -> Program {
        Program {
            main,
            functions: Vec::new(),
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        }
    }

    #[test]
    fn translates_simple_arithmetic_program() {
        // 1 + 2 * 3; Return
        let main = chunk_from(
            vec![
                Op::Const(0),
                Op::Const(1),
                Op::Const(2),
                Op::Mul,
                Op::Add,
                Op::Return,
            ],
            vec![HostValue::Int(1), HostValue::Int(2), HostValue::Int(3)],
        );
        let program = program_from(main);
        let blob = compile_to_rzbc(&program, "thumbv7em-none-eabihf").expect("should translate");

        let mut out = [Instr::Return; 16];
        let count = rzbc_serde::decode(&blob, &mut out).expect("should decode");
        assert_eq!(
            &out[..count],
            &[
                Instr::PushConst(RtValue::Int(1)),
                Instr::PushConst(RtValue::Int(2)),
                Instr::PushConst(RtValue::Int(3)),
                Instr::Mul,
                Instr::Add,
                Instr::Return,
            ]
        );

        let mut vm = resilient_runtime::vm::Vm::<8, 0>::new();
        assert_eq!(vm.run(&out[..count]), Ok(RtValue::Int(7)));
    }

    #[test]
    fn translates_loop_with_jumps_preserving_targets() {
        // Mirrors resilient-runtime's own
        // `loop_sums_one_to_five_via_jump` test program, but built
        // from `Op`'s *relative* jump encoding to exercise
        // `jump_target`'s offset math.
        let main = chunk_from(
            vec![
                Op::Const(0),       // 0: push 0
                Op::StoreLocal(0),  // 1: i = 0
                Op::Const(0),       // 2: push 0
                Op::StoreLocal(1),  // 3: sum = 0
                Op::LoadLocal(0),   // 4: loop: push i
                Op::Const(1),       // 5: push 5
                Op::Lt,             // 6: i < 5
                Op::JumpIfFalse(9), // 7: -> end (target = 8 + 9 = 17)
                Op::LoadLocal(1),   // 8: push sum
                Op::LoadLocal(0),   // 9: push i
                Op::Add,            // 10: sum + i
                Op::StoreLocal(1),  // 11: sum = ...
                Op::LoadLocal(0),   // 12: push i
                Op::Const(2),       // 13: push 1
                Op::Add,            // 14: i + 1
                Op::StoreLocal(0),  // 15: i = ...
                Op::Jump(-13),      // 16: -> loop (target = 17 - 13 = 4)
                Op::LoadLocal(1),   // 17: end: push sum
                Op::Return,         // 18
            ],
            vec![HostValue::Int(0), HostValue::Int(5), HostValue::Int(1)],
        );
        let program = program_from(main);
        let blob = compile_to_rzbc(&program, "riscv32imac-unknown-none-elf")
            .expect("should translate loop");

        let mut out = [Instr::Return; 32];
        let count = rzbc_serde::decode(&blob, &mut out).expect("should decode");

        let mut vm = resilient_runtime::vm::Vm::<8, 2>::new();
        assert_eq!(
            vm.run(&out[..count]),
            Ok(RtValue::Int(1 + 2 + 3 + 4)),
            "translated loop program should compute the same sum as \
             resilient_runtime::vm's own hand-written loop test"
        );
    }

    fn function_from(name: &str, arity: u8, local_count: u16, chunk: Chunk) -> Function {
        Function {
            name: name.to_string(),
            arity,
            chunk,
            local_count,
            upvalue_source_slots: Box::default(),
            fails: Box::default(),
            postcheck: None,
        }
    }

    // RES-4077 (D-E1 fn-support): this test used to be
    // `rejects_fn_declarations` — the whole point of RES-4077 is to
    // make `fn` declarations translate instead of being rejected, so
    // it's rewritten to prove the positive case (compiles, decodes,
    // and executes correctly on `resilient_runtime::vm::Vm`) rather
    // than deleted. The three `rejects_*` tests immediately below
    // cover the fn-shaped constructs that remain out of scope (see
    // the module docs): closures, checked failures, and
    // postcondition-check functions.
    #[test]
    fn compiles_and_executes_top_level_fn_declarations() {
        // fn square(x: Int) -> Int { x * x }
        // main: square(6)
        let square = chunk_from(
            vec![
                Op::LoadLocal(0),
                Op::LoadLocal(0),
                Op::Mul,
                Op::ReturnFromCall,
            ],
            vec![],
        );
        let main = chunk_from(
            vec![Op::Const(0), Op::Call(0), Op::Return],
            vec![HostValue::Int(6)],
        );
        let program = Program {
            main,
            functions: vec![function_from("square", 1, 1, square)],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };

        let blob =
            compile_to_rzbc(&program, "thumbv6m-none-eabi").expect("fn decls should translate");

        let mut out_main = [Instr::Return; 8];
        let mut out_func_meta = [rzbc_serde::DecodedFunctionMeta {
            offset: 0,
            len: 0,
            arity: 0,
            local_count: 0,
            postcheck: None,
        }; 4];
        let mut out_func_code = [Instr::Return; 16];
        let counts = rzbc_serde::decode_program(
            &blob,
            &mut out_main,
            &mut out_func_meta,
            &mut out_func_code,
        )
        .expect("should decode as the function-table format");
        assert_eq!(counts.func_count, 1);

        let meta = out_func_meta[0];
        assert_eq!(meta.arity, 1);
        assert_eq!(meta.local_count, 1);
        assert_eq!(meta.postcheck, None);
        let functions = [resilient_runtime::vm::FunctionDef {
            code: &out_func_code[meta.offset as usize..(meta.offset + meta.len) as usize],
            arity: meta.arity,
            local_count: meta.local_count,
            postcheck: meta.postcheck,
        }];
        let mut vm = resilient_runtime::vm::Vm::<8, 4, 2>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &out_main[..counts.main_len]),
            Ok(RtValue::Int(36))
        );
    }

    #[test]
    fn rejects_closures_capturing_upvalues() {
        let mut closure = function_from("f", 0, 0, chunk_from(vec![Op::ReturnFromCall], vec![]));
        closure.upvalue_source_slots = vec![0u16].into_boxed_slice();
        let program = Program {
            main: chunk_from(vec![Op::Return], vec![]),
            functions: vec![closure],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let err = compile_to_rzbc(&program, "thumbv6m-none-eabi").unwrap_err();
        assert!(err.reason.contains("upvalue"), "reason was: {}", err.reason);
    }

    #[test]
    fn rejects_functions_declaring_fails() {
        let mut fallible = function_from("f", 0, 0, chunk_from(vec![Op::ReturnFromCall], vec![]));
        fallible.fails = vec!["Overflow".to_string()].into_boxed_slice();
        let program = Program {
            main: chunk_from(vec![Op::Return], vec![]),
            functions: vec![fallible],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let err = compile_to_rzbc(&program, "thumbv6m-none-eabi").unwrap_err();
        assert!(err.reason.contains("fails"), "reason was: {}", err.reason);
    }

    // RES-4083 (D-E1 tail): postcheck-bearing functions used to be
    // rejected here (`rejects_functions_with_postcheck`); the
    // embedded VM now runs the postcheck as an isolated nested call
    // on `Instr::Return` (mirroring the host's `run_postcheck`), so
    // this proves the positive case instead.
    #[test]
    fn compiles_and_executes_fn_with_postcheck() {
        // fn f(x: Int) -> Int { x + 1 } ensures result > 0
        // postcheck(x: Int, result: Int) -> Bool { result > 0 }
        // main: f(5) == 6
        let f_body = chunk_from(
            vec![Op::LoadLocal(0), Op::Const(0), Op::Add, Op::ReturnFromCall],
            vec![HostValue::Int(1)],
        );
        let postcheck_body = chunk_from(
            vec![Op::LoadLocal(1), Op::Const(0), Op::Gt, Op::ReturnFromCall],
            vec![HostValue::Int(0)],
        );
        let mut f = function_from("f", 1, 1, f_body);
        f.postcheck = Some(1);
        let postcheck = function_from("f$postcheck", 2, 2, postcheck_body);
        let main = chunk_from(
            vec![Op::Const(0), Op::Call(0), Op::Return],
            vec![HostValue::Int(5)],
        );
        let program = Program {
            main,
            functions: vec![f, postcheck],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };

        let blob = compile_to_rzbc(&program, "thumbv6m-none-eabi")
            .expect("postcheck-bearing fn should translate");

        let mut out_main = [Instr::Return; 8];
        let mut out_func_meta = [rzbc_serde::DecodedFunctionMeta {
            offset: 0,
            len: 0,
            arity: 0,
            local_count: 0,
            postcheck: None,
        }; 4];
        let mut out_func_code = [Instr::Return; 16];
        let counts = rzbc_serde::decode_program(
            &blob,
            &mut out_main,
            &mut out_func_meta,
            &mut out_func_code,
        )
        .expect("should decode as the function-table format");
        assert_eq!(counts.func_count, 2);
        assert_eq!(out_func_meta[0].postcheck, Some(1));
        assert_eq!(out_func_meta[1].postcheck, None);

        let functions: Vec<resilient_runtime::vm::FunctionDef<'_>> = out_func_meta
            [..counts.func_count]
            .iter()
            .map(|meta| resilient_runtime::vm::FunctionDef {
                code: &out_func_code[meta.offset as usize..(meta.offset + meta.len) as usize],
                arity: meta.arity,
                local_count: meta.local_count,
                postcheck: meta.postcheck,
            })
            .collect();
        let mut vm = resilient_runtime::vm::Vm::<8, 4, 4>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &out_main[..counts.main_len]),
            Ok(RtValue::Int(6))
        );
    }

    #[test]
    fn compiles_and_executes_fn_with_violated_postcheck() {
        // fn f(x: Int) -> Int { x } ensures result > 0 — violated when x <= 0.
        let f_body = chunk_from(vec![Op::LoadLocal(0), Op::ReturnFromCall], vec![]);
        let postcheck_body = chunk_from(
            vec![Op::LoadLocal(1), Op::Const(0), Op::Gt, Op::ReturnFromCall],
            vec![HostValue::Int(0)],
        );
        let mut f = function_from("f", 1, 1, f_body);
        f.postcheck = Some(1);
        let postcheck = function_from("f$postcheck", 2, 2, postcheck_body);
        let main = chunk_from(
            vec![Op::Const(0), Op::Call(0), Op::Return],
            vec![HostValue::Int(-1)],
        );
        let program = Program {
            main,
            functions: vec![f, postcheck],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };

        let blob = compile_to_rzbc(&program, "thumbv6m-none-eabi")
            .expect("postcheck-bearing fn should translate");

        let mut out_main = [Instr::Return; 8];
        let mut out_func_meta = [rzbc_serde::DecodedFunctionMeta {
            offset: 0,
            len: 0,
            arity: 0,
            local_count: 0,
            postcheck: None,
        }; 4];
        let mut out_func_code = [Instr::Return; 16];
        let counts = rzbc_serde::decode_program(
            &blob,
            &mut out_main,
            &mut out_func_meta,
            &mut out_func_code,
        )
        .expect("should decode as the function-table format");

        let functions: Vec<resilient_runtime::vm::FunctionDef<'_>> = out_func_meta
            [..counts.func_count]
            .iter()
            .map(|meta| resilient_runtime::vm::FunctionDef {
                code: &out_func_code[meta.offset as usize..(meta.offset + meta.len) as usize],
                arity: meta.arity,
                local_count: meta.local_count,
                postcheck: meta.postcheck,
            })
            .collect();
        let mut vm = resilient_runtime::vm::Vm::<8, 4, 4>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &out_main[..counts.main_len]),
            Err(resilient_runtime::vm::VmError::PostcheckViolation)
        );
    }

    #[test]
    fn rejects_string_constants() {
        let main = chunk_from(
            vec![Op::Const(0), Op::Return],
            vec![HostValue::String("hi".to_string())],
        );
        let program = program_from(main);
        let err = compile_to_rzbc(&program, "thumbv7em-none-eabihf").unwrap_err();
        assert!(
            err.reason.contains("Int/Bool/Float"),
            "reason was: {}",
            err.reason
        );
    }

    #[test]
    fn rejects_unsupported_opcodes() {
        let main = chunk_from(vec![Op::IncLocal(0), Op::Return], vec![]);
        let program = program_from(main);
        let err = compile_to_rzbc(&program, "thumbv7em-none-eabihf").unwrap_err();
        assert!(
            err.reason.contains("IncLocal"),
            "reason was: {}",
            err.reason
        );

        // RES-4075: `Op::Pop` graduated into the supported subset,
        // so the second unsupported-opcode probe is now a bitwise op.
        let main = chunk_from(vec![Op::Band, Op::Return], vec![]);
        let program = program_from(main);
        let err = compile_to_rzbc(&program, "thumbv7em-none-eabihf").unwrap_err();
        assert!(err.reason.contains("Band"), "reason was: {}", err.reason);
    }

    /// RES-4075 (fn-support tail): `Op::Pop` (discarded expression
    /// statements) and `Op::TailCall` (the peephole's self-recursion
    /// rewrite) now translate instead of erroring.
    #[test]
    fn translates_pop_and_tail_call() {
        let main = chunk_from(
            vec![Op::Const(0), Op::Call(0), Op::Pop, Op::Const(0), Op::Return],
            vec![HostValue::Int(1)],
        );
        let body = chunk_from(vec![Op::LoadLocal(0), Op::TailCall(0)], vec![]);
        let program = Program {
            main,
            functions: vec![function_from("f", 1, 1, body)],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        // Just proving it emits — execution semantics are covered by
        // resilient-runtime's own TailCall/Pop tests and the
        // rzbc_build_roundtrip end-to-end tests.
        compile_to_rzbc(&program, "thumbv6m-none-eabi")
            .expect("Pop and TailCall should be emittable");
    }

    #[test]
    fn rejects_out_of_range_jump_target() {
        // Offset that would underflow below index 0.
        let main = chunk_from(vec![Op::Jump(-5), Op::Return], vec![]);
        let program = program_from(main);
        let err = compile_to_rzbc(&program, "thumbv7em-none-eabihf").unwrap_err();
        assert!(
            err.reason.contains("jump target"),
            "reason was: {}",
            err.reason
        );
    }
}
