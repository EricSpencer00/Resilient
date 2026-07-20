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
//! - (RES-4083, host closure emission) a nested `fn`/closure literal
//!   that captures outer locals *by value, never reassigned after
//!   capture* translates to [`Instr::MakeClosure`]/[`Instr::CallClosure`]
//!   against the embedded VM's fixed-capacity capture slab (see
//!   [`resilient_runtime::vm::MAX_CLOSURE_CAPTURES`]). The host compiler
//!   always boxes a captured local into a shared `Value::Cell` — even
//!   when the source program never mutates it — so every read/write of
//!   a captured name compiles to `Op::CallMethod("get"/"set", ..)`
//!   rather than a plain `Op::LoadLocal`/`Op::StoreLocal`. This module's
//!   [`transform_ops`] recognizes and elides the box-conversion
//!   (`LoadLocal; CallBuiltin("cell", 1); StoreLocal`) and unwrap
//!   (`LoadLocal; CallMethod("get", 0)`) idioms as no-ops (sound
//!   because nothing ever *writes* through the cell — see below), and
//!   remaps the closure body's param/capture local-slot ranges to the
//!   embedded call-frame layout (captures first, then call arguments),
//!   stripping the host-only upvalue copy-in prologue that has no
//!   embedded equivalent. Any `Op::CallMethod("set", 1)` found anywhere
//!   — the *only* thing the host compiler ever boxes a local for is a
//!   closure capture, so this is unconditionally a write to a captured
//!   variable after the closure captured it — is a typed [`EmitError`]:
//!   the embedded VM's captures are `Copy` snapshots taken once at
//!   `MakeClosure` time, with no live-shared cell to keep them
//!   consistent with a later host-side mutation, so that divergence is
//!   rejected rather than silently emulated. Both a closure call
//!   through a named local and an inline/anonymous callee expression
//!   are supported, with any number of call-site arguments:
//!   [`bridge_closure_call_args`] finds each `Op::CallClosure`'s
//!   argument-evaluation block and its callee-evaluation block by
//!   static operand-stack-effect accounting ([`op_stack_effect`]) and
//!   rotates the callee block to sit directly before the call, so the
//!   host's `[closure, arg0, .., argN]` push order becomes the
//!   `[arg0, .., argN, closure]` order [`Instr::CallClosure`] expects
//!   (it pops the closure first, then `arity` args in reverse). A
//!   callee or argument expression built from an opcode this bridge's
//!   stack-effect table doesn't cover is a typed [`EmitError`], not a
//!   miscompile. A closure capturing more than
//!   [`MAX_CLOSURE_CAPTURES`] values, or capturing a `static` binding,
//!   is also a typed [`EmitError`].
//! - (RES-4083, D-E1 tail) a function declaring `fails` is
//!   supported: `func.fails[0]`'s variant name is interned to a
//!   numeric id (see [`build_variant_map`]) and carried as
//!   [`resilient_runtime::vm::FunctionDef::fails_variant`] — the
//!   embedded VM's `Instr::Call` deterministically injects it inside
//!   an active `try` block, mirroring the host VM's `h_call`. A
//!   `try { } catch Variant { }` block's `Op::EnterTry`/`Op::ExitTry`
//!   translate to `Instr::EnterTry`/`Instr::ExitTry` against a
//!   *global* try-handler table this module flattens from every
//!   chunk's own `Chunk::try_handlers` (see [`flatten_try_handlers`])
//!   — the embedded wire format has one flat table rather than one
//!   per chunk, so `Op::EnterTry(local_idx)` is rebased by each
//!   chunk's offset into that global table. A single `try` block
//!   declaring more than [`resilient_runtime::vm::MAX_CATCH_ARMS`]
//!   catch arms is a typed [`EmitError`] (the embedded `TryHandlerEntry`
//!   is a fixed-size array, not a `Vec`).
//! - (RES-4083, D-E1 tail) a synthesized postcondition-check
//!   function (`ensures`/`recovers_to`) is supported: the embedded
//!   `Instr::Return` now invokes the postcheck function — itself
//!   just another `program.functions` entry — as an isolated nested
//!   call, mirroring the host VM's `run_postcheck`. See
//!   `resilient_runtime::vm::FunctionDef::postcheck`.
//! - Every function/`main` [`Op`] must be one this module knows how
//!   to translate 1:1 into an [`Instr`] (see [`translate_chunk`]).
//!   Anything else — `IncLocal`, arrays, structs, enums,
//!   closures, FFI, builtins, bitwise ops,
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
use crate::bytecode::{Chunk, Op, Program, TryHandlerEntry as HostTryHandlerEntry};
use resilient_runtime::vm::serde as rzbc_serde;
use resilient_runtime::vm::serde::EncodeFunctionDef;
use resilient_runtime::vm::{
    CatchArm as RtCatchArm, Instr, MAX_CATCH_ARMS, MAX_CLOSURE_CAPTURES,
    TryHandlerEntry as RtTryHandlerEntry, Value as RtValue,
};
use std::collections::HashMap;

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
    // RES-4083 (D-E1 tail): intern every `fails`/`catch` variant name
    // in the whole program to a stable numeric id up front, so
    // `func.fails[0]` and every `catch Variant` arm referencing the
    // same name agree on the same id regardless of translation order.
    let variant_map = build_variant_map(program);
    // RES-4083 (closure call-site arguments): `Op::Call(idx)` doesn't
    // carry its own arity — the callee's declared arity is what the
    // VM pops — so `bridge_closure_call_args`'s stack-effect
    // accounting needs this table to walk backward across a `Call`
    // inside a closure call's argument or callee expression.
    let arities: Vec<u16> = program.functions.iter().map(|f| f.arity as u16).collect();

    let mut global_try_handlers: Vec<RtTryHandlerEntry> = Vec::new();
    let main_try_base = global_try_handlers.len();
    let main_handlers = flatten_try_handlers(&program.main.try_handlers, &variant_map, target)?;
    global_try_handlers.extend(main_handlers);
    let main_instrs =
        translate_chunk_transformed(&program.main, target, main_try_base, None, &arities)?;

    if program.functions.is_empty() {
        if !global_try_handlers.is_empty() {
            return Err(unsupported(
                target,
                "top-level `try { }` blocks require the function-table `.rzbc` format, but this \
                 program declares no functions — a `try` around nothing but builtin/expression \
                 code has no `fails`-declaring callee to ever dispatch a catch arm for"
                    .to_string(),
            ));
        }
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
    let mut fails_variants: Vec<Option<u16>> = Vec::with_capacity(program.functions.len());
    let mut capture_counts: Vec<u8> = Vec::with_capacity(program.functions.len());
    for func in &program.functions {
        let shift = if func.upvalue_source_slots.is_empty() {
            None
        } else {
            if func.upvalue_source_slots.len() > MAX_CLOSURE_CAPTURES {
                return Err(unsupported(
                    target,
                    format!(
                        "function `{}` captures {} value(s), exceeding the embedded VM's fixed \
                         capacity of {MAX_CLOSURE_CAPTURES} scalar captures per closure",
                        func.name,
                        func.upvalue_source_slots.len()
                    ),
                ));
            }
            if func.upvalue_source_slots.contains(&u16::MAX) {
                return Err(unsupported(
                    target,
                    format!(
                        "function `{}` captures a `static` binding — the embedded translator only \
                         supports scalar-by-value captures of ordinary locals (RES-4083)",
                        func.name
                    ),
                ));
            }
            Some((func.arity as u16, func.upvalue_source_slots.len() as u16))
        };
        capture_counts.push(shift.map(|(_, cc)| cc as u8).unwrap_or(0));

        // RES-4083 (D-E1 tail): the host VM only ever injects
        // `func.fails[0]` on a checked-failure dispatch (see
        // `vm.rs`'s `h_call`), so only that first variant needs a
        // carried id — `build_variant_map` already interned it.
        fails_variants.push(func.fails.first().map(|v| variant_map[v]));

        let try_base = global_try_handlers.len();
        let handlers = flatten_try_handlers(&func.chunk.try_handlers, &variant_map, target)?;
        global_try_handlers.extend(handlers);

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
        func_instrs.push(translate_chunk_transformed(
            &func.chunk,
            target,
            try_base,
            shift,
            &arities,
        )?);
    }

    let functions: Vec<EncodeFunctionDef<'_>> = program
        .functions
        .iter()
        .zip(func_instrs.iter())
        .zip(fails_variants.iter())
        .zip(capture_counts.iter())
        .map(
            |(((func, instrs), fails_variant), capture_count)| EncodeFunctionDef {
                code: instrs.as_slice(),
                arity: func.arity,
                local_count: func.local_count,
                postcheck: func.postcheck,
                fails_variant: *fails_variant,
                capture_count: *capture_count,
            },
        )
        .collect();

    let func_instr_total: usize = func_instrs.iter().map(Vec::len).sum();
    // Each try-handler entry, worst case: 1 (arm_count) +
    // MAX_CATCH_ARMS * (2 variant + 4 handler_pc) bytes.
    const MAX_TRY_ENTRY_WIRE_WIDTH: usize = 1 + MAX_CATCH_ARMS * (2 + 4);
    let cap = rzbc_serde::HEADER_LEN
        + (main_instrs.len() + func_instr_total + functions.len()) * MAX_INSTR_WIRE_WIDTH
        + functions.len() * 8
        + global_try_handlers.len() * MAX_TRY_ENTRY_WIRE_WIDTH
        + 2;
    let mut buf = vec![0u8; cap];
    let len = rzbc_serde::encode_program(&main_instrs, &functions, &global_try_handlers, &mut buf)
        .map_err(|e| {
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

/// RES-4083 (D-E1 tail): intern every `fails`-declaring function's
/// first variant name and every `catch Variant` arm's name (across
/// `program.main` and every `program.functions` entry) into a single
/// `name -> id` map, assigning ids in first-seen order. Both a
/// function's `fails_variant` and its catch arms' `variant` ids come
/// from this same map, so `f() fails Boom` and `catch Boom { }`
/// always agree on the numeric id regardless of which chunk defines
/// which — the embedded wire format has no string constant pool to
/// carry the name itself (see the module docs' "narrow bridge"
/// section), so this id is the only representation that survives.
fn build_variant_map(program: &Program) -> HashMap<String, u16> {
    let mut map: HashMap<String, u16> = HashMap::new();
    let intern = |name: &str, map: &mut HashMap<String, u16>| {
        if !map.contains_key(name) {
            let id = map.len() as u16;
            map.insert(name.to_string(), id);
        }
    };
    for entry in &program.main.try_handlers {
        for arm in &entry.arms {
            intern(&arm.variant, &mut map);
        }
    }
    for func in &program.functions {
        if let Some(first) = func.fails.first() {
            intern(first, &mut map);
        }
        for entry in &func.chunk.try_handlers {
            for arm in &entry.arms {
                intern(&arm.variant, &mut map);
            }
        }
    }
    map
}

/// RES-4083 (D-E1 tail): translate one chunk's `Chunk::try_handlers`
/// table into the embedded [`RtTryHandlerEntry`] shape, resolving
/// each arm's variant name through `variant_map`. A variant name with
/// no id in `variant_map` can't happen — every name reachable from a
/// `catch` arm was interned by [`build_variant_map`] scanning the
/// same `try_handlers` tables — but the lookup still goes through
/// `HashMap::get` rather than indexing, so a hypothetical future
/// caller mismatch is a clean panic-free path, not a panic.
fn flatten_try_handlers(
    handlers: &[HostTryHandlerEntry],
    variant_map: &HashMap<String, u16>,
    target: &str,
) -> Result<Vec<RtTryHandlerEntry>, EmitError> {
    handlers
        .iter()
        .map(|entry| {
            if entry.arms.len() > MAX_CATCH_ARMS {
                return Err(unsupported(
                    target,
                    format!(
                        "a `try` block declares {} `catch` arms, exceeding the embedded no_std \
                         VM's fixed limit of {MAX_CATCH_ARMS} arms per `try` block",
                        entry.arms.len()
                    ),
                ));
            }
            let mut arms = [None; MAX_CATCH_ARMS];
            for (slot, arm) in arms.iter_mut().zip(entry.arms.iter()) {
                let variant = *variant_map.get(&arm.variant).ok_or_else(|| {
                    unsupported(
                        target,
                        format!(
                            "internal error: catch arm variant `{}` has no interned id — this is \
                             a bug in rzbc_emit's variant table, not a property of the source \
                             program",
                            arm.variant
                        ),
                    )
                })?;
                let handler_pc = u32::try_from(arm.handler_pc).map_err(|_| {
                    unsupported(
                        target,
                        format!(
                            "catch arm handler pc {} does not fit the `.rzbc` format's u32 \
                             absolute-target encoding",
                            arm.handler_pc
                        ),
                    )
                })?;
                *slot = Some(RtCatchArm {
                    variant,
                    handler_pc,
                });
            }
            Ok(RtTryHandlerEntry { arms })
        })
        .collect()
}

/// Translate every [`Op`] in `chunk.code` to the matching [`Instr`],
/// 1:1 by index (`out[i]` is the translation of `chunk.code[i]`) —
/// see the module docs for why this index-preserving property is
/// exactly what makes [`jump_target`]'s offset-to-absolute-index math
/// sound.
fn translate_chunk(chunk: &Chunk, target: &str, try_base: usize) -> Result<Vec<Instr>, EmitError> {
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
            // RES-4083 (D-E1 tail): `handler_table` is a *local*
            // index into this chunk's own `Chunk::try_handlers` —
            // rebase it by `try_base` (this chunk's offset into the
            // flattened global table `compile_to_rzbc` built via
            // `flatten_try_handlers`) so `Instr::EnterTry` indexes
            // correctly into that global table at runtime.
            Op::EnterTry(handler_table) => {
                let global_idx = try_base
                    .checked_add(handler_table as usize)
                    .and_then(|idx| u16::try_from(idx).ok())
                    .ok_or_else(|| {
                        unsupported(
                            target,
                            format!(
                                "internal error: global try-handler index overflowed u16 (chunk-\
                                 local index {handler_table}, base {try_base}) — this is a bug in \
                                 rzbc_emit, not a property of the source program"
                            ),
                        )
                    })?;
                Instr::EnterTry(global_idx)
            }
            Op::ExitTry => Instr::ExitTry,
            // RES-4083 (host closure emission): `upvalue_count` was
            // already bounds-checked against `MAX_CLOSURE_CAPTURES` in
            // `compile_to_rzbc` before this chunk was translated.
            Op::MakeClosure {
                fn_idx,
                upvalue_count,
            } => Instr::MakeClosure {
                func_idx: fn_idx,
                capture_count: upvalue_count,
            },
            // RES-4083 (closure call-site arguments): any call-site
            // argument list, and any callee expression shape (a named
            // local *or* an inline/anonymous closure expression), is
            // supported now — [`bridge_closure_call_args`] already
            // rotated the callee-expression ops to sit directly before
            // this op by the time `transform_ops` hands the chunk to
            // this function, so the operand stack is already in the
            // `[..., arg0, .., argN, closure]` shape
            // [`resilient_runtime::vm::Instr::CallClosure`] expects —
            // this arm has nothing left to validate.
            Op::CallClosure { .. } => Instr::CallClosure,
            // Everything else is (b)-class or otherwise absent from
            // `Instr` (bitwise ops, `IncLocal`,
            // FFI, builtins, arrays/structs/enums/tuples,
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

/// RES-4083 (host closure emission): [`transform_ops`] `chunk.code`
/// (de-boxing captures and, for a closure body, stripping the upvalue
/// prologue + remapping local slots), then hand the result to
/// [`translate_chunk`] via a shallow clone that swaps in the
/// transformed code (constants/`try_handlers` are untouched, so
/// `try_base` rebasing still applies correctly).
///
/// `shift` is `Some((arity, capture_count))` when `chunk` is itself a
/// closure body (see [`transform_ops`]), `None` for an ordinary chunk
/// (`main`, or a plain top-level `fn`) that may merely *define* a
/// closure (and so still needs the box/unbox idiom eliding, just not
/// the prologue strip / slot remap).
fn translate_chunk_transformed(
    chunk: &Chunk,
    target: &str,
    try_base: usize,
    shift: Option<(u16, u16)>,
    arities: &[u16],
) -> Result<Vec<Instr>, EmitError> {
    let transformed_code = transform_ops(&chunk.code, chunk, target, shift, arities)?;
    let mut transformed_chunk = chunk.clone();
    transformed_chunk.code = transformed_code;
    translate_chunk(&transformed_chunk, target, try_base)
}

/// RES-4083 (host closure emission): find the constant-pool index of
/// the `Value::String(s)` entry in `chunk.constants`, or `None` if `s`
/// was never interned into this chunk (e.g. a chunk with no closures
/// at all never adds `"cell"`/`"get"`/`"set"`).
fn find_string_const(chunk: &Chunk, s: &str) -> Option<u16> {
    chunk
        .constants
        .iter()
        .position(|v| matches!(v, HostValue::String(x) if x.as_str() == s))
        .map(|i| i as u16)
}

/// RES-4083 (closure call-site arguments): net operand-stack effect
/// (values pushed minus values popped) of a single `Op`, covering
/// exactly the opcodes [`translate_chunk`] accepts in this no_std
/// subset plus `MakeClosure`/`CallClosure` — used only by
/// [`bridge_closure_call_args`]'s backward scan to locate the
/// boundary between a closure call's argument block, its callee
/// expression, and whatever precedes both. Returns `None` for any
/// other opcode; the caller turns that into a typed [`EmitError`]
/// rather than guessing at an unknown effect.
fn op_stack_effect(op: &Op, arities: &[u16]) -> Option<i32> {
    Some(match *op {
        Op::Const(_) | Op::LoadLocal(_) | Op::LoadUpvalue(_) => 1,
        Op::StoreLocal(_) | Op::StoreUpvalue { .. } | Op::Pop => -1,
        Op::Add
        | Op::Sub
        | Op::Mul
        | Op::Div
        | Op::Mod
        | Op::Eq
        | Op::Neq
        | Op::Lt
        | Op::Le
        | Op::Gt
        | Op::Ge => -1,
        Op::Neg | Op::Not | Op::IncLocal(_) | Op::Jump(_) | Op::EnterTry(_) | Op::ExitTry => 0,
        Op::JumpIfFalse(_) | Op::JumpIfTrue(_) => -1,
        Op::MakeClosure { upvalue_count, .. } => 1 - upvalue_count as i32,
        // Pops the closure plus `arity` args, pushes one result —
        // regardless of `source_slot`, which only affects upvalue
        // write-back on the host, not the stack shape.
        Op::CallClosure { arity, .. } => -(arity as i32),
        Op::Call(idx) => 1 - *arities.get(idx as usize)? as i32,
        _ => return None,
    })
}

/// RES-4083 (closure call-site arguments): rewrite `code` so every
/// `Op::CallClosure { arity, .. }` with `arity > 0` has its callee
/// expression's ops moved to sit directly before it, turning the host
/// compiler's `[closure_expr…, arg0_expr…, .., argN_expr…]` evaluation
/// order into `[arg0_expr…, .., argN_expr…, closure_expr…]` — the
/// order [`resilient_runtime::vm::Instr::CallClosure`] needs, since it
/// pops the closure value first and the args after. This is sound
/// because a *rotation* of a contiguous op range changes no op's
/// stack effect and, since every `Jump*` inside the rotated range
/// still targets another instruction inside the same range at the
/// same *relative* distance (both source and destination shift left
/// by the same amount), no jump offset needs recomputing either.
///
/// The boundary between "callee expression", "argument block", and
/// "the rest of the chunk" is found by walking backward from the
/// `CallClosure` op and summing [`op_stack_effect`]: the argument
/// block is the shortest suffix (ending just before `CallClosure`)
/// whose total effect is exactly `arity` (each of the `arity`
/// arguments leaves exactly one value on the stack), and the callee
/// expression is the next block back whose total effect is exactly
/// `1` (it must leave exactly the closure value). An opcode this
/// function's stack-effect table doesn't cover, or a boundary that
/// can't be found this way, is a typed [`EmitError`] — never a silent
/// miscompile.
fn bridge_closure_call_args(
    code: &[Op],
    arities: &[u16],
    target: &str,
) -> Result<Vec<Op>, EmitError> {
    let mut code = code.to_vec();
    let mut call_idx = 0;
    while call_idx < code.len() {
        let Op::CallClosure { arity, source_slot } = code[call_idx] else {
            call_idx += 1;
            continue;
        };
        if arity == 0 {
            call_idx += 1;
            continue;
        }
        let target_arity = arity as i32;

        let find_boundary = |from: usize, want: i32, what: &str| -> Result<usize, EmitError> {
            let mut acc = 0i32;
            let mut k = from;
            while k > 0 {
                k -= 1;
                let eff = op_stack_effect(&code[k], arities).ok_or_else(|| {
                    unsupported(
                        target,
                        format!(
                            "a closure call's {what} uses opcode `{:?}` that the embedded \
                             call-site argument bridge doesn't understand (RES-4083)",
                            code[k]
                        ),
                    )
                })?;
                acc += eff;
                if acc == want {
                    return Ok(k);
                }
            }
            Err(unsupported(
                target,
                format!(
                    "internal error: could not locate the start of this closure call's {what} by \
                     stack-effect accounting — this is a bug in rzbc_emit's closure-call bridge, \
                     not a property of the source program"
                ),
            ))
        };

        let args_start = find_boundary(call_idx, target_arity, "argument block")?;
        let closure_start = find_boundary(args_start, 1, "callee expression")?;

        if source_slot != u16::MAX
            && !(closure_start + 1 == args_start
                && matches!(code[closure_start], Op::LoadLocal(slot) if slot == source_slot))
        {
            return Err(unsupported(
                target,
                format!(
                    "closure call at index {call_idx} expected a plain `LoadLocal({source_slot})` \
                     as its whole callee expression but found a more complex shape — the embedded \
                     call-site argument bridge only understands the exact shape `compiler.rs`'s \
                     named-local closure-call path emits (RES-4083)"
                ),
            ));
        }

        code[closure_start..call_idx].rotate_left(args_start - closure_start);
        call_idx += 1;
    }
    Ok(code)
}

/// RES-4083 (host closure emission): rewrite `code` so it is safe and
/// ready for [`translate_chunk`] to translate 1:1:
///
/// 1. Reject (typed [`EmitError`]) any `Op::CallMethod("set", 1)` —
///    `compiler.rs`'s `BOXED_FLAG` is only ever set on a captured
///    local (see `analyze_and_box_captures`/`compile_assignment`), so
///    this opcode unconditionally means "a captured variable was
///    written to" — i.e. reassignment after capture, which the
///    embedded VM's `Copy`-snapshot captures can't soundly emulate
///    (see the module docs' closures section).
/// 2. Elide the box-conversion idiom
///    `LoadLocal(x); CallBuiltin("cell", 1); StoreLocal(x)` (a no-op
///    once (1) has proven the boxed cell is never mutated — boxing
///    then never reading-back-through-a-mutation is behaviorally
///    identical to never boxing at all).
/// 3. Elide the unwrap idiom `LoadLocal(x); CallMethod("get", 0)` down
///    to a plain `LoadLocal(x)`, for the same reason.
/// 4. If `shift` is `Some((arity, capture_count))` (`code` is a
///    closure body), verify the leading `2 * capture_count` ops are
///    exactly the `LoadUpvalue(i); StoreLocal(arity + i)` copy-in
///    prologue `compiler.rs`'s `install_upvalue_locals_and_prologue`
///    emits, strip them (the embedded call-frame VM already seeds
///    `locals[0..capture_count]` from the closure's captures at call
///    time — see `Instr::CallClosure` — so there is nothing left to
///    copy in), and remap every remaining `LoadLocal`/`StoreLocal`/
///    `IncLocal` index from the host's `[params][captures]` local
///    layout to the embedded VM's `[captures][params]` layout.
///
/// Every `Op::Jump`/`JumpIfFalse`/`JumpIfTrue`'s relative offset is
/// recomputed against the transformed (possibly shorter) instruction
/// count in a second pass, so [`jump_target`]'s index-preserving
/// assumption still holds for the array this function returns.
fn transform_ops(
    code: &[Op],
    chunk: &Chunk,
    target: &str,
    shift: Option<(u16, u16)>,
    arities: &[u16],
) -> Result<Vec<Op>, EmitError> {
    let bridged_code = bridge_closure_call_args(code, arities, target)?;
    let code: &[Op] = &bridged_code;

    let get_idx = find_string_const(chunk, "get");
    let set_idx = find_string_const(chunk, "set");
    let cell_idx = find_string_const(chunk, "cell");

    for op in code {
        if let Op::CallMethod {
            method_const,
            arity: 1,
        } = *op
            && Some(method_const) == set_idx
        {
            return Err(unsupported(
                target,
                "a captured variable is written to after the closure that captures it was \
                 created — the embedded VM's closures are scalar by-value captures that are \
                 never reassigned after capture, so this divergence from the host's live-shared \
                 `Cell` semantics can't be soundly emulated (RES-4083)"
                    .to_string(),
            ));
        }
    }

    let prologue_len = shift.map(|(_, cc)| cc as usize * 2).unwrap_or(0);
    if code.len() < prologue_len {
        return Err(unsupported(
            target,
            "closure body is shorter than its expected upvalue copy-in prologue — this is a bug \
             in rzbc_emit's closure translation, not a property of the source program"
                .to_string(),
        ));
    }
    if let Some((arity, _)) = shift {
        for (j, pair) in code[..prologue_len].chunks_exact(2).enumerate() {
            let j = j as u16;
            // RES-4083 (host closure emission): `compiler.rs`'s
            // `rewrite_store_upvalues` pass runs *after* the prologue is
            // emitted and rewrites any `Op::StoreLocal` targeting the
            // upvalue pseudo-local range into `Op::StoreUpvalue` — the
            // prologue's own copy-in store is no exception, so it
            // arrives here as `StoreUpvalue`, not the raw `StoreLocal`
            // `install_upvalue_locals_and_prologue`'s doc comment
            // describes emitting.
            let ok = matches!(
                (pair[0], pair[1]),
                (Op::LoadUpvalue(idx), Op::StoreUpvalue { upvalue_idx, local_slot })
                    if idx == j && upvalue_idx == j && local_slot == arity + j
            );
            if !ok {
                return Err(unsupported(
                    target,
                    format!(
                        "closure body's upvalue copy-in prologue has an unexpected shape at pair \
                         {j} ({:?}, {:?}) — the embedded translator only understands the exact \
                         prologue `compiler.rs`'s `install_upvalue_locals_and_prologue` emits",
                        pair[0], pair[1]
                    ),
                ));
            }
        }
    }

    let remap_slot = |idx: u16| -> u16 {
        match shift {
            Some((arity, capture_count)) if idx < arity => capture_count + idx,
            Some((arity, capture_count)) if idx < arity + capture_count => idx - arity,
            _ => idx,
        }
    };

    let n = code.len();
    let mut old_to_new: Vec<u32> = vec![0; n + 1];
    let mut new_code: Vec<Op> = Vec::with_capacity(n);
    let mut jump_fixups: Vec<(usize, i64)> = Vec::new();

    let mut i = prologue_len;
    while i < n {
        if let Op::LoadLocal(x) = code[i] {
            if i + 1 < n
                && matches!(
                    code[i + 1],
                    Op::CallMethod { method_const, arity: 0 } if Some(method_const) == get_idx
                )
            {
                let new_i = new_code.len() as u32;
                new_code.push(Op::LoadLocal(remap_slot(x)));
                old_to_new[i] = new_i;
                old_to_new[i + 1] = new_i;
                i += 2;
                continue;
            }
            if i + 2 < n
                && matches!(
                    code[i + 1],
                    Op::CallBuiltin { name_const, arity: 1 } if Some(name_const) == cell_idx
                )
                && matches!(code[i + 2], Op::StoreLocal(y) if y == x)
            {
                let new_i = new_code.len() as u32;
                old_to_new[i] = new_i;
                old_to_new[i + 1] = new_i;
                old_to_new[i + 2] = new_i;
                i += 3;
                continue;
            }
        }
        let new_i = new_code.len() as u32;
        old_to_new[i] = new_i;
        match code[i] {
            Op::LoadLocal(x) => new_code.push(Op::LoadLocal(remap_slot(x))),
            Op::StoreLocal(x) => new_code.push(Op::StoreLocal(remap_slot(x))),
            Op::IncLocal(x) => new_code.push(Op::IncLocal(remap_slot(x))),
            Op::Jump(offset) => {
                jump_fixups.push((new_code.len(), i as i64 + 1 + offset as i64));
                new_code.push(Op::Jump(0));
            }
            Op::JumpIfFalse(offset) => {
                jump_fixups.push((new_code.len(), i as i64 + 1 + offset as i64));
                new_code.push(Op::JumpIfFalse(0));
            }
            Op::JumpIfTrue(offset) => {
                jump_fixups.push((new_code.len(), i as i64 + 1 + offset as i64));
                new_code.push(Op::JumpIfTrue(0));
            }
            other => new_code.push(other),
        }
        i += 1;
    }
    old_to_new[n] = new_code.len() as u32;

    for (new_i, old_target) in jump_fixups {
        if old_target < 0 || old_target as usize > n {
            return Err(unsupported(
                target,
                format!(
                    "jump target {old_target} does not fall within this chunk's {n} \
                     instructions — the source program's own jump offset is out of range"
                ),
            ));
        }
        let new_target = old_to_new[old_target as usize] as i64;
        let new_offset = new_target - (new_i as i64 + 1);
        let offset16 = i16::try_from(new_offset).map_err(|_| {
            unsupported(
                target,
                "a jump offset overflowed i16 after closure-capture translation — this is a bug \
                 in rzbc_emit's closure translation, not a property of the source program"
                    .to_string(),
            )
        })?;
        match &mut new_code[new_i] {
            Op::Jump(o) | Op::JumpIfFalse(o) | Op::JumpIfTrue(o) => *o = offset16,
            _ => unreachable!("jump_fixups only ever records Jump*-kind ops"),
        }
    }

    Ok(new_code)
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
            fails_variant: None,
            capture_count: 0,
        }; 4];
        let mut out_func_code = [Instr::Return; 16];
        let mut out_try_handlers = [resilient_runtime::vm::TryHandlerEntry::EMPTY; 1];
        let counts = rzbc_serde::decode_program(
            &blob,
            &mut out_main,
            &mut out_func_meta,
            &mut out_func_code,
            &mut out_try_handlers,
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
            fails_variant: meta.fails_variant,
            capture_count: 0,
        }];
        let mut vm = resilient_runtime::vm::Vm::<8, 4, 2, 0, 0, 8>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &out_main[..counts.main_len]),
            Ok(RtValue::Int(36))
        );
    }

    // RES-4083 (host closure emission): a nested `fn`/closure literal
    // capturing an outer local by value, never reassigned after
    // capture, now translates via `Instr::MakeClosure`/`CallClosure`.
    // This mirrors exactly what `compiler.rs::compile_nested_fn`
    // produces for:
    //   let base = 41;
    //   fn getBase() { base }
    //   getBase()
    #[test]
    fn compiles_and_executes_zero_arg_closure_over_one_capture() {
        let get_base = chunk_from(
            vec![
                Op::LoadUpvalue(0),
                Op::StoreUpvalue {
                    upvalue_idx: 0,
                    local_slot: 0,
                },
                Op::LoadLocal(0),
                Op::CallMethod {
                    method_const: 0,
                    arity: 0,
                },
                Op::ReturnFromCall,
            ],
            vec![HostValue::String("get".to_string())],
        );
        let mut get_base_fn = function_from("getBase", 0, 1, get_base);
        get_base_fn.upvalue_source_slots = vec![0u16].into_boxed_slice();

        let main = chunk_from(
            vec![
                Op::Const(0),      // 0: push 41
                Op::StoreLocal(0), // 1: base = 41
                Op::LoadLocal(0),  // 2: box base ->
                Op::CallBuiltin {
                    name_const: 1,
                    arity: 1,
                }, // 3:   cell(base)
                Op::StoreLocal(0), // 4:   base = Cell(41)
                Op::LoadLocal(0),  // 5: push captured Cell handle
                Op::MakeClosure {
                    fn_idx: 0,
                    upvalue_count: 1,
                }, // 6: f = closure(getBase, [base])
                Op::StoreLocal(1), // 7: f = ...
                Op::LoadLocal(1),  // 8: push f
                Op::CallClosure {
                    arity: 0,
                    source_slot: 1,
                }, // 9: f()
                Op::Return,        // 10
            ],
            vec![HostValue::Int(41), HostValue::String("cell".to_string())],
        );

        let program = Program {
            main,
            functions: vec![get_base_fn],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };

        let blob = compile_to_rzbc(&program, "thumbv6m-none-eabi")
            .expect("zero-arg closure over one by-value capture should translate");

        let mut out_main = [Instr::Return; 16];
        let mut out_func_meta = [rzbc_serde::DecodedFunctionMeta {
            offset: 0,
            len: 0,
            arity: 0,
            local_count: 0,
            postcheck: None,
            fails_variant: None,
            capture_count: 0,
        }; 4];
        let mut out_func_code = [Instr::Return; 16];
        let mut out_try_handlers = [resilient_runtime::vm::TryHandlerEntry::EMPTY; 1];
        let counts = rzbc_serde::decode_program(
            &blob,
            &mut out_main,
            &mut out_func_meta,
            &mut out_func_code,
            &mut out_try_handlers,
        )
        .expect("should decode as the function-table format");
        assert_eq!(counts.func_count, 1);

        let meta = out_func_meta[0];
        assert_eq!(meta.arity, 0);
        assert_eq!(meta.local_count, 1);
        assert_eq!(meta.capture_count, 1);
        let functions = [resilient_runtime::vm::FunctionDef {
            code: &out_func_code[meta.offset as usize..(meta.offset + meta.len) as usize],
            arity: meta.arity,
            local_count: meta.local_count,
            postcheck: meta.postcheck,
            fails_variant: meta.fails_variant,
            capture_count: meta.capture_count,
        }];
        let mut vm = resilient_runtime::vm::Vm::<8, 4, 2, 0, 2, 8>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &out_main[..counts.main_len]),
            Ok(RtValue::Int(41))
        );
    }

    #[test]
    fn rejects_closure_capturing_more_than_max_captures() {
        let mut closure = function_from("f", 0, 0, chunk_from(vec![Op::ReturnFromCall], vec![]));
        closure.upvalue_source_slots =
            vec![0u16; resilient_runtime::vm::MAX_CLOSURE_CAPTURES + 1].into_boxed_slice();
        let program = Program {
            main: chunk_from(vec![Op::Return], vec![]),
            functions: vec![closure],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let err = compile_to_rzbc(&program, "thumbv6m-none-eabi").unwrap_err();
        assert!(
            err.reason.contains("exceeding"),
            "reason was: {}",
            err.reason
        );
    }

    #[test]
    fn compiles_and_executes_closure_called_with_arguments() {
        // RES-4083 (closure call-site arguments): a closure called
        // with call-site arguments now bridges the host's
        // `[closure, arg…]` push order to the embedded VM's
        // `[arg…, closure]` pop order:
        //   let base = 41;
        //   fn addBase(int x) { base + x }
        //   addBase(1)  // -> 42
        let add_base = chunk_from(
            vec![
                Op::LoadUpvalue(0),
                Op::StoreUpvalue {
                    upvalue_idx: 0,
                    local_slot: 1,
                },
                Op::LoadLocal(1), // captured base (boxed)
                Op::CallMethod {
                    method_const: 0,
                    arity: 0,
                }, // .get()
                Op::LoadLocal(0), // x
                Op::Add,
                Op::ReturnFromCall,
            ],
            vec![HostValue::String("get".to_string())],
        );
        let mut add_base_fn = function_from("addBase", 1, 2, add_base);
        add_base_fn.upvalue_source_slots = vec![0u16].into_boxed_slice();

        let main = chunk_from(
            vec![
                Op::Const(0),      // 0: push 41
                Op::StoreLocal(0), // 1: base = 41
                Op::LoadLocal(0),  // 2: box base ->
                Op::CallBuiltin {
                    name_const: 1,
                    arity: 1,
                }, // 3:   cell(base)
                Op::StoreLocal(0), // 4:   base = Cell(41)
                Op::LoadLocal(0),  // 5: push captured Cell handle
                Op::MakeClosure {
                    fn_idx: 0,
                    upvalue_count: 1,
                }, // 6: f = closure(addBase, [base])
                Op::StoreLocal(1), // 7: f = ...
                Op::LoadLocal(1),  // 8: push f
                Op::Const(2),      // 9: push 1
                Op::CallClosure {
                    arity: 1,
                    source_slot: 1,
                }, // 10: f(1)
                Op::Return,        // 11
            ],
            vec![
                HostValue::Int(41),
                HostValue::String("cell".to_string()),
                HostValue::Int(1),
            ],
        );

        let program = Program {
            main,
            functions: vec![add_base_fn],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };

        let blob = compile_to_rzbc(&program, "thumbv6m-none-eabi")
            .expect("closure called with a call-site argument should translate");

        let mut out_main = [Instr::Return; 16];
        let mut out_func_meta = [rzbc_serde::DecodedFunctionMeta {
            offset: 0,
            len: 0,
            arity: 0,
            local_count: 0,
            postcheck: None,
            fails_variant: None,
            capture_count: 0,
        }; 4];
        let mut out_func_code = [Instr::Return; 16];
        let mut out_try_handlers = [resilient_runtime::vm::TryHandlerEntry::EMPTY; 1];
        let counts = rzbc_serde::decode_program(
            &blob,
            &mut out_main,
            &mut out_func_meta,
            &mut out_func_code,
            &mut out_try_handlers,
        )
        .expect("should decode as the function-table format");
        assert_eq!(counts.func_count, 1);

        let meta = out_func_meta[0];
        assert_eq!(meta.arity, 1);
        assert_eq!(meta.capture_count, 1);
        let functions = [resilient_runtime::vm::FunctionDef {
            code: &out_func_code[meta.offset as usize..(meta.offset + meta.len) as usize],
            arity: meta.arity,
            local_count: meta.local_count,
            postcheck: meta.postcheck,
            fails_variant: meta.fails_variant,
            capture_count: meta.capture_count,
        }];
        let mut vm = resilient_runtime::vm::Vm::<8, 4, 2, 0, 2, 8>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &out_main[..counts.main_len]),
            Ok(RtValue::Int(42))
        );
    }

    #[test]
    fn rejects_closure_call_argument_using_unsupported_opcode() {
        // Same shape as `compiles_and_executes_closure_called_with_arguments`
        // but the argument expression is a bitwise op that the
        // embedded call-site argument bridge's stack-effect table
        // doesn't cover — a typed `EmitError`, not a miscompile.
        let add_base = chunk_from(vec![Op::LoadLocal(0), Op::ReturnFromCall], vec![]);
        let mut add_base_fn = function_from("addBase", 1, 1, add_base);
        add_base_fn.upvalue_source_slots = vec![].into_boxed_slice();
        let main = chunk_from(
            vec![
                Op::Const(0),
                Op::StoreLocal(0),
                Op::LoadLocal(0),
                Op::Const(1),
                Op::Const(2),
                Op::Band,
                Op::CallClosure {
                    arity: 1,
                    source_slot: 0,
                },
                Op::Return,
            ],
            vec![HostValue::Int(0), HostValue::Int(1), HostValue::Int(2)],
        );
        let program = Program {
            main,
            functions: vec![add_base_fn],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let err = compile_to_rzbc(&program, "thumbv6m-none-eabi").unwrap_err();
        assert!(
            err.reason.contains("closure call's argument block"),
            "reason was: {}",
            err.reason
        );
    }

    #[test]
    fn rejects_capture_reassigned_after_closure_creation() {
        // let base = 41; fn getBase() { base } base = 99; getBase()
        // — `base = 99` after the closure captured it compiles to
        // `Op::CallMethod("set", 1)`, which is unconditionally
        // rejected (see `transform_ops`'s doc comment).
        let get_base = chunk_from(
            vec![
                Op::LoadUpvalue(0),
                Op::StoreUpvalue {
                    upvalue_idx: 0,
                    local_slot: 0,
                },
                Op::LoadLocal(0),
                Op::CallMethod {
                    method_const: 0,
                    arity: 0,
                },
                Op::ReturnFromCall,
            ],
            vec![HostValue::String("get".to_string())],
        );
        let mut get_base_fn = function_from("getBase", 0, 1, get_base);
        get_base_fn.upvalue_source_slots = vec![0u16].into_boxed_slice();

        let main = chunk_from(
            vec![
                Op::Const(0),
                Op::StoreLocal(0),
                Op::LoadLocal(0),
                Op::CallBuiltin {
                    name_const: 1,
                    arity: 1,
                },
                Op::StoreLocal(0),
                Op::LoadLocal(0),
                Op::MakeClosure {
                    fn_idx: 0,
                    upvalue_count: 1,
                },
                Op::StoreLocal(1),
                // base = 99 (post-capture reassignment):
                Op::LoadLocal(0),
                Op::Const(2),
                Op::CallMethod {
                    method_const: 3,
                    arity: 1,
                },
                Op::StoreLocal(2),
                Op::LoadLocal(1),
                Op::CallClosure {
                    arity: 0,
                    source_slot: 1,
                },
                Op::Return,
            ],
            vec![
                HostValue::Int(41),
                HostValue::String("cell".to_string()),
                HostValue::Int(99),
                HostValue::String("set".to_string()),
            ],
        );
        let program = Program {
            main,
            functions: vec![get_base_fn],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let err = compile_to_rzbc(&program, "thumbv6m-none-eabi").unwrap_err();
        assert!(
            err.reason.contains("reassigned after capture")
                || err.reason.contains("written to after"),
            "reason was: {}",
            err.reason
        );
    }

    // RES-4083 (D-E1 tail): `fails`-declaring functions used to be
    // rejected here (`rejects_functions_declaring_fails`); the
    // embedded VM now dispatches a checked failure through a
    // translated `try`/`catch` block (mirroring the host's `h_call`
    // injection), so this proves the positive case instead.
    #[test]
    fn compiles_and_executes_fn_with_fails_and_try_catch() {
        // fn risky() fails Boom { 1 }
        // main: try { risky(); 0 } catch Boom { -1 }
        let risky = chunk_from(
            vec![Op::Const(0), Op::ReturnFromCall],
            vec![HostValue::Int(1)],
        );
        let mut fallible = function_from("risky", 0, 0, risky);
        fallible.fails = vec!["Boom".to_string()].into_boxed_slice();

        let mut main = chunk_from(
            vec![
                Op::EnterTry(0), // 0
                Op::Call(0),     // 1: risky() — never runs its body
                Op::Pop,         // 2: discard the (never produced) result
                Op::Const(0),    // 3: push 0
                Op::Jump(2),     // 4: -> Return (skip the catch arm)
                Op::Const(1),    // 5: catch Boom -> push -1
                Op::Return,      // 6
                Op::ExitTry,     // 7
                Op::Return,      // 8
            ],
            vec![HostValue::Int(0), HostValue::Int(-1)],
        );
        main.try_handlers.push(crate::bytecode::TryHandlerEntry {
            arms: vec![crate::bytecode::CatchArm {
                variant: "Boom".to_string(),
                handler_pc: 5,
            }],
        });

        let program = Program {
            main,
            functions: vec![fallible],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };

        let blob = compile_to_rzbc(&program, "thumbv6m-none-eabi")
            .expect("fails/try/catch should translate");

        let mut out_main = [Instr::Return; 16];
        let mut out_func_meta = [rzbc_serde::DecodedFunctionMeta {
            offset: 0,
            len: 0,
            arity: 0,
            local_count: 0,
            postcheck: None,
            fails_variant: None,
            capture_count: 0,
        }; 4];
        let mut out_func_code = [Instr::Return; 16];
        let mut out_try_handlers = [resilient_runtime::vm::TryHandlerEntry::EMPTY; 4];
        let counts = rzbc_serde::decode_program(
            &blob,
            &mut out_main,
            &mut out_func_meta,
            &mut out_func_code,
            &mut out_try_handlers,
        )
        .expect("should decode as the function-table format");
        assert_eq!(counts.func_count, 1);
        assert_eq!(counts.try_count, 1);
        assert_eq!(out_func_meta[0].fails_variant, Some(0));

        let functions = [resilient_runtime::vm::FunctionDef {
            code: &out_func_code[out_func_meta[0].offset as usize
                ..(out_func_meta[0].offset + out_func_meta[0].len) as usize],
            arity: out_func_meta[0].arity,
            local_count: out_func_meta[0].local_count,
            postcheck: out_func_meta[0].postcheck,
            fails_variant: out_func_meta[0].fails_variant,
            capture_count: 0,
        }];
        let mut vm = resilient_runtime::vm::Vm::<8, 4, 2, 1, 0, 8>::new();
        assert_eq!(
            vm.run_with_tries(
                &functions,
                &out_try_handlers[..counts.try_count],
                &out_main[..counts.main_len]
            ),
            Ok(RtValue::Int(-1))
        );
    }

    #[test]
    fn rejects_try_block_with_too_many_catch_arms() {
        let mut main = chunk_from(vec![Op::EnterTry(0), Op::Return], vec![]);
        main.try_handlers.push(crate::bytecode::TryHandlerEntry {
            arms: (0..(resilient_runtime::vm::MAX_CATCH_ARMS + 1))
                .map(|i| crate::bytecode::CatchArm {
                    variant: format!("V{i}"),
                    handler_pc: 0,
                })
                .collect(),
        });
        let program = program_from(main);
        let err = compile_to_rzbc(&program, "thumbv6m-none-eabi").unwrap_err();
        assert!(err.reason.contains("catch"), "reason was: {}", err.reason);
    }

    #[test]
    fn rejects_top_level_try_with_no_functions() {
        let mut main = chunk_from(vec![Op::EnterTry(0), Op::ExitTry, Op::Return], vec![]);
        main.try_handlers.push(crate::bytecode::TryHandlerEntry {
            arms: vec![crate::bytecode::CatchArm {
                variant: "Boom".to_string(),
                handler_pc: 1,
            }],
        });
        let program = program_from(main);
        let err = compile_to_rzbc(&program, "thumbv6m-none-eabi").unwrap_err();
        assert!(
            err.reason.contains("function-table"),
            "reason was: {}",
            err.reason
        );
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
            fails_variant: None,
            capture_count: 0,
        }; 4];
        let mut out_func_code = [Instr::Return; 16];
        let mut out_try_handlers = [resilient_runtime::vm::TryHandlerEntry::EMPTY; 1];
        let counts = rzbc_serde::decode_program(
            &blob,
            &mut out_main,
            &mut out_func_meta,
            &mut out_func_code,
            &mut out_try_handlers,
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
                fails_variant: meta.fails_variant,
                capture_count: 0,
            })
            .collect();
        let mut vm = resilient_runtime::vm::Vm::<8, 4, 4, 0, 0, 16>::new();
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
            fails_variant: None,
            capture_count: 0,
        }; 4];
        let mut out_func_code = [Instr::Return; 16];
        let mut out_try_handlers = [resilient_runtime::vm::TryHandlerEntry::EMPTY; 1];
        let counts = rzbc_serde::decode_program(
            &blob,
            &mut out_main,
            &mut out_func_meta,
            &mut out_func_code,
            &mut out_try_handlers,
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
                fails_variant: meta.fails_variant,
                capture_count: 0,
            })
            .collect();
        let mut vm = resilient_runtime::vm::Vm::<8, 4, 4, 0, 0, 16>::new();
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
