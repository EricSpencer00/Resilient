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
//! [`Instr`] only has dispatch arms for the no_std-clean subset
//! minus bitwise ops (not yet ported to `Instr`). RES-4075 added
//! function calls: `main` plus every function chunk is laid out
//! into one flat instruction stream (main at index 0, functions
//! appended in table order), with a v2 `.rzbc` function table
//! (entry pc / arity / local count) that the embedded VM's
//! fixed-capacity call-frame stack indexes via
//! `Instr::Call`/`Ret`/`TailCall`. This module enforces exactly
//! that subset at compile time:
//!
//! - Functions must be plain: no closures (`upvalue_source_slots`),
//!   no `fails` variants, no `ensures`/`recovers_to` postcheck —
//!   each is a typed [`EmitError`] naming the offending `fn`.
//! - Every chunk's every [`Op`] must be one this module knows
//!   how to translate 1:1 into an [`Instr`] (see [`translate_chunk`]).
//!   Anything else — `IncLocal`, arrays, structs, enums,
//!   closures, try/catch, FFI, builtins, bitwise ops, ... — is a
//!   typed [`EmitError`] naming the exact opcode, never a silently
//!   malformed blob.
//! - Every `Op::Const` constant must be `Value::Int`/`Bool`/`Float`
//!   — `Instr::PushConst` carries the value inline (no separate
//!   constant pool in the `.rzbc` format; see
//!   `resilient_runtime::vm::serde`'s module docs for the wire
//!   layout this mirrors), so a `String`/`Array`/... constant has no
//!   representation to translate into.
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
use resilient_runtime::vm::{FnEntry, Instr, Value as RtValue};

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
pub fn compile_to_rzbc(program: &Program, target: &str) -> Result<Vec<u8>, EmitError> {
    // RES-4075: functions are supported (fixed-capacity call frames
    // on the embedded VM) — with the v1 exclusions below, each of
    // which names a construct the embedded VM has no representation
    // for yet.
    for f in &program.functions {
        if !f.upvalue_source_slots.is_empty() {
            return Err(unsupported(
                target,
                format!(
                    "fn `{}` is a closure (captures {} upvalue(s)) — the embedded no_std VM's \
                     call frames carry no captured-environment slab",
                    f.name,
                    f.upvalue_source_slots.len()
                ),
            ));
        }
        if !f.fails.is_empty() {
            return Err(unsupported(
                target,
                format!(
                    "fn `{}` declares `fails` variants — try/checked-failure semantics are not \
                     part of the embedded no_std VM subset",
                    f.name
                ),
            ));
        }
        if f.postcheck.is_some() {
            return Err(unsupported(
                target,
                format!(
                    "fn `{}` has an `ensures`/`recovers_to` postcondition-check function — \
                     runtime contract checks are not part of the embedded no_std VM subset",
                    f.name
                ),
            ));
        }
    }

    // Layout: main's instructions first (index 0 is the entrypoint),
    // then each function's chunk appended in table order. Jump
    // targets inside each chunk are chunk-local, so every chunk is
    // translated with its own base offset.
    let mut instrs = translate_chunk(&program.main, 0, target)?;
    let mut fns = Vec::with_capacity(program.functions.len());
    for f in &program.functions {
        let entry = instrs.len();
        let entry_u32 = u32::try_from(entry).map_err(|_| {
            unsupported(
                target,
                format!(
                    "fn `{}` starts at instruction {}, past the `.rzbc` format's u32 entry-point \
                     encoding",
                    f.name, entry
                ),
            )
        })?;
        let local_count = f.local_count.max(f.arity as u16);
        fns.push(FnEntry {
            entry: entry_u32,
            arity: f.arity,
            local_count,
        });
        instrs.extend(translate_chunk(&f.chunk, entry, target)?);
    }
    let main_local_count = max_local_count(&program.main);

    let cap = rzbc_serde::PROGRAM_HEADER_LEN
        + fns.len() * rzbc_serde::FN_ENTRY_LEN
        + instrs.len() * MAX_INSTR_WIRE_WIDTH;
    let mut buf = vec![0u8; cap];
    let len =
        rzbc_serde::encode_program(&instrs, &fns, main_local_count, &mut buf).map_err(|e| {
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

/// How many locals slots top-level code needs: one past the highest
/// local index `main` touches. Written into the v2 header so the
/// embedded VM knows where call frames may start carving the slab.
fn max_local_count(chunk: &Chunk) -> u16 {
    let mut count: u16 = 0;
    for op in &chunk.code {
        if let Op::LoadLocal(idx) | Op::StoreLocal(idx) | Op::IncLocal(idx) = *op {
            count = count.max(idx.saturating_add(1));
        }
    }
    count
}

/// Translate every [`Op`] in `chunk.code` to the matching [`Instr`],
/// 1:1 by index (`out[i]` is the translation of `chunk.code[i]`) —
/// see the module docs for why this index-preserving property is
/// exactly what makes [`jump_target`]'s offset-to-absolute-index math
/// sound.
fn translate_chunk(chunk: &Chunk, base: usize, target: &str) -> Result<Vec<Instr>, EmitError> {
    let mut out = Vec::with_capacity(chunk.code.len());
    for (i, op) in chunk.code.iter().enumerate() {
        let instr = match *op {
            Op::Const(idx) => Instr::PushConst(translate_const(chunk, idx, target)?),
            Op::Pop => Instr::Pop,
            Op::Call(idx) => Instr::Call(idx),
            Op::TailCall(idx) => Instr::TailCall(idx),
            Op::ReturnFromCall => Instr::Ret,
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
            Op::Jump(offset) => Instr::Jump(jump_target(i, base, offset, target)?),
            Op::JumpIfFalse(offset) => Instr::JumpIfFalse(jump_target(i, base, offset, target)?),
            Op::JumpIfTrue(offset) => Instr::JumpIfTrue(jump_target(i, base, offset, target)?),
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
/// *after* the jump, at index `i + 1` within its chunk) into the
/// `.rzbc` format's absolute `u32` instruction index. `base` is the
/// chunk's start offset in the flat concatenated stream (0 for
/// `main`; each function's entry pc otherwise — RES-4075). Sound
/// only because [`translate_chunk`] emits exactly one `Instr` per
/// `Op`, so chunk-local index `i` and stream index `base + i`
/// always refer to the same instruction.
fn jump_target(i: usize, base: usize, offset: i16, target: &str) -> Result<u32, EmitError> {
    let pc_after = i as i64 + 1;
    let dest = base as i64 + pc_after + offset as i64;
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

    const NO_FN: FnEntry = FnEntry {
        entry: 0,
        arity: 0,
        local_count: 0,
    };

    fn plain_fn(
        name: &str,
        arity: u8,
        local_count: u16,
        chunk: Chunk,
    ) -> crate::bytecode::Function {
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
        let mut fns = [NO_FN; 4];
        let header = rzbc_serde::decode_program(&blob, &mut out, &mut fns).expect("should decode");
        let count = header.instr_count;
        assert_eq!(header.fn_count, 0);
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
        let mut fns = [NO_FN; 4];
        let header = rzbc_serde::decode_program(&blob, &mut out, &mut fns).expect("should decode");
        let count = header.instr_count;

        let mut vm = resilient_runtime::vm::Vm::<8, 2>::new();
        assert_eq!(
            vm.run(&out[..count]),
            Ok(RtValue::Int(1 + 2 + 3 + 4)),
            "translated loop program should compute the same sum as \
             resilient_runtime::vm's own hand-written loop test"
        );
    }

    /// RES-4075: replaces `rejects_fn_declarations` — fn
    /// declarations now compile to a v2 blob whose function table
    /// and relocated call/return instructions round-trip through
    /// the real embedded decoder and VM.
    #[test]
    fn compiles_fn_declarations_and_executes_calls() {
        // main: add2(3, 4)   fn add2(a, b) = a + b
        let main = chunk_from(
            vec![Op::Const(0), Op::Const(1), Op::Call(0), Op::Return],
            vec![HostValue::Int(3), HostValue::Int(4)],
        );
        let body = chunk_from(
            vec![
                Op::LoadLocal(0),
                Op::LoadLocal(1),
                Op::Add,
                Op::ReturnFromCall,
            ],
            vec![],
        );
        let program = Program {
            main,
            functions: vec![plain_fn("add2", 2, 2, body)],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let blob = compile_to_rzbc(&program, "thumbv6m-none-eabi").expect("fns should compile");

        let mut out = [Instr::Return; 16];
        let mut fns = [NO_FN; 4];
        let header = rzbc_serde::decode_program(&blob, &mut out, &mut fns).expect("should decode");
        assert_eq!(header.fn_count, 1);
        assert_eq!(
            fns[0],
            FnEntry {
                entry: 4, // main has 4 instructions; fn body starts right after
                arity: 2,
                local_count: 2
            }
        );

        let mut vm = resilient_runtime::vm::Vm::<8, 8, 4>::new();
        assert_eq!(
            vm.run_program(
                &out[..header.instr_count],
                &fns[..header.fn_count],
                header.main_local_count
            ),
            Ok(RtValue::Int(7))
        );
    }

    /// RES-4075: a function body's chunk-local jump offsets must be
    /// rebased onto the fn's position in the flat stream.
    #[test]
    fn rebases_jump_targets_inside_function_bodies() {
        // fn pick(flag) = if flag { 10 } else { 20 }; main: pick(false)
        let main = chunk_from(
            vec![Op::Const(0), Op::Call(0), Op::Return],
            vec![HostValue::Bool(false)],
        );
        let body = chunk_from(
            vec![
                Op::LoadLocal(0),   // fn-local 0 (stream 3)
                Op::JumpIfFalse(2), // -> fn-local 4 (stream 7)
                Op::Const(0),       // 10
                Op::ReturnFromCall, // fn-local 3
                Op::Const(1),       // fn-local 4: 20
                Op::ReturnFromCall,
            ],
            vec![HostValue::Int(10), HostValue::Int(20)],
        );
        let program = Program {
            main,
            functions: vec![plain_fn("pick", 1, 1, body)],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let blob = compile_to_rzbc(&program, "thumbv7em-none-eabihf").expect("should compile");

        let mut out = [Instr::Return; 16];
        let mut fns = [NO_FN; 4];
        let header = rzbc_serde::decode_program(&blob, &mut out, &mut fns).expect("should decode");
        assert_eq!(
            out[4],
            Instr::JumpIfFalse(7),
            "target must be rebased by entry pc 3"
        );

        let mut vm = resilient_runtime::vm::Vm::<8, 8, 4>::new();
        assert_eq!(
            vm.run_program(
                &out[..header.instr_count],
                &fns[..header.fn_count],
                header.main_local_count
            ),
            Ok(RtValue::Int(20))
        );
    }

    #[test]
    fn rejects_closures_fails_and_postchecks() {
        let mk = |f: Function| Program {
            main: chunk_from(vec![Op::Const(0), Op::Return], vec![HostValue::Int(0)]),
            functions: vec![f],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };

        let mut closure = plain_fn("c", 0, 0, Chunk::new());
        closure.upvalue_source_slots = vec![0u16].into_boxed_slice();
        let err = compile_to_rzbc(&mk(closure), "thumbv6m-none-eabi").unwrap_err();
        assert!(err.reason.contains("closure"), "reason was: {}", err.reason);

        let mut fails = plain_fn("f", 0, 0, Chunk::new());
        fails.fails = vec!["Overflow".to_string()].into_boxed_slice();
        let err = compile_to_rzbc(&mk(fails), "thumbv6m-none-eabi").unwrap_err();
        assert!(err.reason.contains("fails"), "reason was: {}", err.reason);

        let mut post = plain_fn("p", 0, 0, Chunk::new());
        post.postcheck = Some(1);
        let err = compile_to_rzbc(&mk(post), "thumbv6m-none-eabi").unwrap_err();
        assert!(err.reason.contains("ensures"), "reason was: {}", err.reason);
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
