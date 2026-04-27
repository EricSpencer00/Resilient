//! RES-365: function inlining pass.
//!
//! Replaces `Op::Call(idx)` to small leaf functions with the callee's
//! bytecode body, eliminating call-frame push/pop overhead and exposing
//! follow-on optimization opportunities (the const-folder and peephole
//! pass can then act across the inlined boundary).
//!
//! ## Inlining heuristic
//!
//! A function is "inlineable" when ALL of the following hold:
//!
//! - Body has ≤ [`INLINE_THRESHOLD`] bytecode ops (excluding the
//!   trailing `ReturnFromCall` tombstone).
//! - No self-recursion: no `Op::Call(self_idx)` and no `Op::TailCall`.
//! - No closures: no `Op::MakeClosure` and no `Op::LoadUpvalue`.
//! - No foreign calls: no `Op::CallForeign` (semantics of resolved
//!   FFI symbols are opaque; conservatively skip).
//!
//! Call-stack depth, recursion termination, and FFI behavior all
//! depend on the call frame existing — inlining would silently change
//! observable behavior in those cases. The check is conservative: a
//! callee that calls some OTHER user function (non-recursive) is fine
//! to inline because that callee's `Call` op survives in the inlined
//! body and runs in the caller's frame, which is exactly what `Call`
//! does anyway.
//!
//! ## Lowering at a call site
//!
//! Given `Op::Call(callee_idx)` where `callee_idx` is inlineable with
//! arity `a` and `local_count` `n`, the rewrite is:
//!
//! 1. Allocate `n` fresh locals at slots `[base..base+n)` in the
//!    caller. This bumps the caller's `local_count` by `n`.
//! 2. The operand stack at the call site holds the args left-to-right
//!    (deepest is leftmost). Pop them right-to-left into
//!    `StoreLocal(base + a-1)` … `StoreLocal(base + 0)` so locals
//!    `0..a` end up holding the args in source order. (This mimics
//!    the VM's `Op::Call` dispatch in `vm.rs`.)
//! 3. Inline the callee body verbatim, with these per-op rewrites:
//!    - `LoadLocal(i)`  → `LoadLocal(base + i)`
//!    - `StoreLocal(i)` → `StoreLocal(base + i)`
//!    - `IncLocal(i)`   → `IncLocal(base + i)` (peephole-introduced)
//!    - `Const(k)`      → `Const(k')` where `k'` interns the value
//!      from the callee's pool into the caller's pool
//!    - `CallBuiltin { name_const, arity }` → same with
//!      `name_const` re-interned
//!    - `StructLiteral { name_const, .. }` / `GetField { name_const }` /
//!      `SetField { name_const }` → same with `name_const` re-interned
//!    - `MakeArray`, `LoadIndex`, `StoreIndex`, arithmetic, comparison,
//!      `Not`, `Neg` — copied verbatim
//!    - `Jump(o)` / `JumpIfFalse(o)` / `JumpIfTrue(o)` — offsets are
//!      relative to the post-jump PC, and the inlined body is a
//!      contiguous slice, so internal offsets stay valid; copied verbatim
//!    - `Call(other_idx)` — copied verbatim (still references the same
//!      function table)
//!    - `ReturnFromCall` — replaced with `Jump(end)` to the end of the
//!      inlined sequence so the result on top of the stack lands
//!      where a normal `Call` would have left it
//!    - `Return` — never appears inside a function body (only in main)
//!    - Trailing `ReturnFromCall` — dropped (we jump to the end anyway)
//!
//! ## Iteration & fixpoint
//!
//! One pass identifies the set of currently-inlineable functions, then
//! rewrites every chunk in the program (each function's chunk plus
//! `main`). Inlining can grow a chunk past the threshold, so a function
//! that was inlineable in pass N may not be in pass N+1. We iterate up
//! to [`MAX_PASSES`] times or until no further inlining fires. The
//! iteration cap is a hard safety net — realistic call graphs converge
//! in 1–3 passes.
//!
//! ## Default-off via env var
//!
//! Inlining changes the bytecode shape of any program that uses
//! function calls. Pre-existing tests pin specific opcode sequences
//! (e.g. `compile_call_emits_call_op` asserts a `Call(0)` survives).
//! Per the project's test-protection policy we cannot modify those
//! tests in this PR, so the pass is gated behind `RESILIENT_INLINE=1`
//! exactly as `const_fold` is gated behind `RESILIENT_CONST_FOLD=1`.

use crate::bytecode::{Chunk, Function, Op, Program};

/// Maximum number of bytecode ops in an inlineable function body
/// (excluding the trailing `ReturnFromCall`). Per the RES-365
/// acceptance criteria the threshold is pinned at 10 — small leaf
/// functions in the typical Resilient program (identity functions,
/// 2-3-op arithmetic helpers, pure projection of a struct field)
/// fit comfortably. Anything larger pays its own way against the
/// call-frame overhead, so inlining ceases to be profitable.
pub const INLINE_THRESHOLD: usize = 10;

/// Hard cap on inlining iteration count. Each pass is O(n × m) where
/// n is total ops across the program and m is the number of call
/// sites. Realistic programs converge in 1–3 passes; this cap exists
/// only to bound runtime if someone constructs a pathological
/// callgraph. Hitting it is an internal bug.
pub const MAX_PASSES: usize = 5;

/// Errors from the inlining pass.
#[derive(Debug)]
pub enum InlineError {
    InternalError(&'static str),
}

impl std::fmt::Display for InlineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InlineError::InternalError(msg) => write!(f, "function inlining: {}", msg),
        }
    }
}

impl std::error::Error for InlineError {}

/// Pipeline-aware entry point. Runs [`optimize`] only when
/// `RESILIENT_INLINE=1`; otherwise returns `Ok(())` without touching
/// the program. Mirrors `const_fold::optimize_if_enabled`.
pub fn optimize_if_enabled(program: &mut Program) -> Result<(), InlineError> {
    if std::env::var("RESILIENT_INLINE").as_deref() == Ok("1") {
        optimize(program)?;
    }
    Ok(())
}

/// Top-level entry. Iterates [`inline_pass`] until no further inlining
/// fires or [`MAX_PASSES`] is reached. Idempotent on programs with no
/// inlineable call sites.
pub fn optimize(program: &mut Program) -> Result<(), InlineError> {
    for _ in 0..MAX_PASSES {
        let inlined_any = inline_pass(program)?;
        if !inlined_any {
            return Ok(());
        }
    }
    Ok(())
}

/// One full pass: identify inlineable functions, then rewrite every
/// chunk in the program. Returns `true` if at least one call site was
/// inlined.
fn inline_pass(program: &mut Program) -> Result<bool, InlineError> {
    // Snapshot inlineable status BEFORE mutating any chunks. Inlining
    // grows the caller's chunk, which could flip its own inlineable
    // status mid-pass and lead to non-deterministic results.
    let inlineable: Vec<bool> = (0..program.functions.len())
        .map(|i| is_inlineable(&program.functions, i as u16))
        .collect();

    // If no function is inlineable, nothing to do.
    if !inlineable.iter().any(|&b| b) {
        return Ok(false);
    }

    // Snapshot every callee chunk + metadata so we can borrow the
    // caller mutably while reading the callee. Cloning is cheap here
    // — chunks are small by definition (≤ INLINE_THRESHOLD ops).
    let callees: Vec<(Function, bool)> = program
        .functions
        .iter()
        .cloned()
        .zip(inlineable.iter().copied())
        .collect();

    let mut inlined_any = false;

    // Inline into each function's chunk.
    for (caller_idx, func) in program.functions.iter_mut().enumerate() {
        let did = inline_into_chunk(
            &mut func.chunk,
            &mut func.local_count,
            &callees,
            Some(caller_idx as u16),
        )?;
        inlined_any |= did;
    }
    // Inline into main. Main has no own_idx (it's not in the function
    // table), so all inlineable callees are fair game.
    let mut main_local_count = main_local_count(&program.main);
    let did = inline_into_chunk(&mut program.main, &mut main_local_count, &callees, None)?;
    inlined_any |= did;

    Ok(inlined_any)
}

/// Decide whether `program.functions[idx]` is inlineable per the
/// heuristic in the module docs. Pure read — no mutation.
fn is_inlineable(functions: &[Function], idx: u16) -> bool {
    let func = match functions.get(idx as usize) {
        Some(f) => f,
        None => return false,
    };
    // Body length excludes the trailing `ReturnFromCall` the compiler
    // unconditionally emits. The chunk may also have a tombstone
    // `Return` from `rewrite_tail_calls`, but for non-recursive
    // candidates (which the next check enforces) that doesn't apply.
    let body_len = body_op_count(&func.chunk);
    if body_len > INLINE_THRESHOLD {
        return false;
    }
    for op in &func.chunk.code {
        match op {
            // Self-recursion via direct or tail call → not a leaf.
            Op::Call(target) if *target == idx => return false,
            Op::TailCall(_) => return false,
            // Closures need an upvalue slab the inliner can't reproduce.
            Op::MakeClosure { .. } | Op::LoadUpvalue(_) => return false,
            // Foreign calls have opaque side effects; skip conservatively.
            Op::CallForeign(_) => return false,
            _ => {}
        }
    }
    true
}

/// Count the "real" body ops in a function chunk — every op up to but
/// excluding the final `ReturnFromCall`. The compiler always emits a
/// trailing `ReturnFromCall` (sometimes preceded by a `Return`
/// tombstone from `rewrite_tail_calls`); for the inline-threshold
/// check we want the size of the user-authored body, not the
/// terminator overhead.
fn body_op_count(chunk: &Chunk) -> usize {
    // Walk back from the end skipping any trailing ReturnFromCall /
    // Return tombstones. There's at most one ReturnFromCall and one
    // Return tombstone (from rewrite_tail_calls); we cap the walk at
    // 2 to avoid mis-counting if the user explicitly wrote a sequence
    // of returns.
    let mut n = chunk.code.len();
    let mut skipped = 0;
    while n > 0 && skipped < 2 {
        match chunk.code[n - 1] {
            Op::ReturnFromCall | Op::Return => {
                n -= 1;
                skipped += 1;
            }
            _ => break,
        }
    }
    n
}

/// Read main's current `local_count` from its chunk. Main doesn't
/// carry an explicit count (only `Function` does), so we infer it
/// from the highest local index any op references. New locals
/// allocated for inlined frames get appended past this.
fn main_local_count(chunk: &Chunk) -> u16 {
    let mut max_idx: i32 = -1;
    for op in &chunk.code {
        let idx = match op {
            Op::LoadLocal(i) | Op::StoreLocal(i) | Op::IncLocal(i) => Some(*i as i32),
            _ => None,
        };
        if let Some(i) = idx
            && i > max_idx
        {
            max_idx = i;
        }
    }
    if max_idx < 0 {
        0
    } else {
        (max_idx as u16).saturating_add(1)
    }
}

/// Inline every eligible call site in `chunk`. Returns `true` if any
/// inlining fired.
///
/// `local_count` is the caller's running local-slot count; on each
/// inlining we bump it by the callee's `local_count` so subsequent
/// inlinings allocate fresh, non-overlapping slots.
///
/// `own_idx` is `Some(i)` when this chunk is for `program.functions[i]`
/// (so we skip self-inlining defensively even though the inlineable
/// check already rejects self-recursive callees), `None` for main.
fn inline_into_chunk(
    chunk: &mut Chunk,
    local_count: &mut u16,
    callees: &[(Function, bool)],
    own_idx: Option<u16>,
) -> Result<bool, InlineError> {
    // Build the new code stream linearly. Internal jump offsets within
    // the inlined body are pre-relative and unchanged; the caller's
    // own jumps need relinking through an old→new PC fixup table the
    // same way peephole.rs / const_fold.rs handle it.
    let orig_targets: Vec<Option<usize>> = chunk
        .code
        .iter()
        .enumerate()
        .map(|(pc, op)| jump_target_pc(*op, pc))
        .collect();

    let mut new_code: Vec<Op> = Vec::with_capacity(chunk.code.len());
    let mut new_lines: Vec<u32> = Vec::with_capacity(chunk.code.len());
    let mut old_to_new: Vec<usize> = vec![usize::MAX; chunk.code.len() + 1];
    let mut inlined_any = false;

    let mut i = 0;
    while i < chunk.code.len() {
        old_to_new[i] = new_code.len();
        if let Op::Call(callee_idx) = chunk.code[i]
            && Some(callee_idx) != own_idx
            && let Some((callee, true)) = callees.get(callee_idx as usize)
        {
            // Allocate fresh slots for the callee's locals.
            let base = *local_count;
            let new_total = (base as u32) + (callee.local_count as u32);
            // Avoid overflowing u16. If this would exceed the cap,
            // fall back to a normal Call rather than corrupt slot
            // numbering. The chunk continues to be valid bytecode.
            if new_total > u16::MAX as u32 {
                new_code.push(chunk.code[i]);
                new_lines.push(chunk.line_info[i]);
                i += 1;
                continue;
            }
            *local_count = new_total as u16;

            let line = chunk.line_info[i];

            // Step 1: pop args off the operand stack into the
            // callee's parameter slots. The VM's Op::Call dispatch
            // pops rightmost-first into locals[i] descending; we
            // mirror that with StoreLocal in reverse order.
            for arg_i in (0..callee.arity as u16).rev() {
                new_code.push(Op::StoreLocal(base + arg_i));
                new_lines.push(line);
            }

            // Step 2: emit the callee body, with op rewrites.
            // Compute the body slice (excluding trailing ReturnFromCall
            // / Return tombstone — same as body_op_count).
            let body_end = body_op_count(&callee.chunk);
            // Reserve the post-inline PC for the Jump-to-end target.
            // We don't know it until all body ops are emitted; track
            // every emitted Jump-replacement-of-ReturnFromCall and
            // back-patch its offset at the end.
            let body_start_new_pc = new_code.len();
            let mut return_jump_patches: Vec<usize> = Vec::new();
            for (body_pc, body_op) in callee.chunk.code[..body_end].iter().enumerate() {
                let body_line = callee.chunk.line_info[body_pc];
                let rewritten = rewrite_inlined_op(*body_op, base, &callee.chunk, chunk)?;
                match rewritten {
                    InlinedOp::Verbatim(op) => {
                        new_code.push(op);
                        new_lines.push(body_line);
                    }
                    InlinedOp::ReturnAsJump => {
                        // Placeholder Jump; we patch the offset
                        // after we know the end-of-body PC.
                        let patch_idx = new_code.len();
                        new_code.push(Op::Jump(0));
                        new_lines.push(body_line);
                        return_jump_patches.push(patch_idx);
                    }
                }
            }
            let body_end_new_pc = new_code.len();
            // Back-patch each ReturnFromCall-replacement Jump so its
            // offset lands on body_end_new_pc.
            for patch_idx in return_jump_patches {
                let pc_after = (patch_idx + 1) as isize;
                let offset = (body_end_new_pc as isize) - pc_after;
                let Ok(offset) = i16::try_from(offset) else {
                    return Err(InlineError::InternalError(
                        "inlined return-jump offset out of i16 range",
                    ));
                };
                if let Op::Jump(o) = &mut new_code[patch_idx] {
                    *o = offset;
                } else {
                    return Err(InlineError::InternalError("back-patch site is not a Jump"));
                }
            }

            // Body internal jumps (Jump/JumpIfFalse/JumpIfTrue inside
            // the inlined region) referenced positions within the
            // original callee chunk. We emitted the body contiguously
            // starting at body_start_new_pc; the offsets in the
            // callee's original code are pre-relative-to-PC-after-op,
            // so as long as we emit verbatim and the body PC layout
            // is preserved (1 op in → 1 op out), the offsets stay
            // valid. rewrite_inlined_op does NOT collapse ops, so
            // the 1:1 layout invariant holds.
            //
            // Sanity check that:
            debug_assert_eq!(
                body_end_new_pc - body_start_new_pc,
                body_end + return_jump_patches_count_check(&callee.chunk.code[..body_end]),
                "inlined body changed op count"
            );

            // Mark the original Call op as having become this whole
            // inlined sequence. Any jump to the original Call PC
            // lands at the start of the inlined sequence (which is
            // semantically equivalent — pushing args and entering
            // the body).
            i += 1;
            inlined_any = true;
            continue;
        }

        // No inlining at this site — copy verbatim.
        new_code.push(chunk.code[i]);
        new_lines.push(chunk.line_info[i]);
        i += 1;
    }
    old_to_new[chunk.code.len()] = new_code.len();

    if !inlined_any {
        return Ok(false);
    }

    // Re-link external jumps in the rewritten stream. Same mechanic
    // as peephole/const_fold: find each new-PC's originating old-PC
    // (only for ops that are jumps and were COPIED VERBATIM from the
    // caller — inlined-body jumps were emitted with their callee-
    // internal offsets and must NOT be relinked through old_to_new).
    //
    // We distinguish caller-jumps from inlined-body-jumps by checking
    // whether the new-PC corresponds to an entry in old_to_new (i.e.
    // some old PC mapped to it). If yes, it's a caller jump; if no,
    // it's an inlined-body op that we leave alone.
    let mut new_pc_owner: Vec<Option<usize>> = vec![None; new_code.len()];
    for (old_pc, &new_pc) in old_to_new.iter().enumerate().take(chunk.code.len()) {
        if new_pc < new_code.len() && new_pc_owner[new_pc].is_none() {
            new_pc_owner[new_pc] = Some(old_pc);
        }
    }

    for (new_pc, op) in new_code.iter_mut().enumerate() {
        if !is_jump_op(*op) {
            continue;
        }
        let Some(old_pc) = new_pc_owner[new_pc] else {
            continue; // inlined-body jump — leave its offset alone
        };
        let Some(old_target) = orig_targets[old_pc] else {
            continue;
        };
        let new_target = old_to_new[old_target];
        let offset = (new_target as isize) - (new_pc as isize + 1);
        let Ok(offset) = i16::try_from(offset) else {
            continue;
        };
        match op {
            Op::Jump(o) => *o = offset,
            Op::JumpIfFalse(o) => *o = offset,
            Op::JumpIfTrue(o) => *o = offset,
            _ => unreachable!("is_jump_op guards the match"),
        }
    }

    chunk.code = new_code;
    chunk.line_info = new_lines;
    Ok(true)
}

/// Sanity-check helper: count the number of ReturnFromCall ops in a
/// callee body slice. Used inside a debug_assert to confirm the
/// inliner's 1:1 op-emission invariant.
fn return_jump_patches_count_check(body: &[Op]) -> usize {
    // The 1:1 invariant: every op in the body slice → exactly one op
    // in new_code. ReturnFromCall maps to a Jump (still 1 op). So
    // total emitted = body.len(), matching the simple identity. The
    // assertion in inline_into_chunk reads:
    //   body_end_new_pc - body_start_new_pc == body_end + 0
    // and this helper returns 0 to make that explicit.
    let _ = body;
    0
}

/// Result of rewriting a single op from a callee body.
enum InlinedOp {
    Verbatim(Op),
    ReturnAsJump,
}

/// Rewrite a single callee-body op for emission into the caller's
/// chunk:
/// - local-slot ops shift by `base`
/// - constant-pool indices remap to caller's pool
/// - `ReturnFromCall` becomes a placeholder Jump
fn rewrite_inlined_op(
    op: Op,
    base: u16,
    callee_chunk: &Chunk,
    caller_chunk: &mut Chunk,
) -> Result<InlinedOp, InlineError> {
    let remap_const = |k: u16, caller: &mut Chunk| -> Result<u16, InlineError> {
        let v = callee_chunk
            .constants
            .get(k as usize)
            .ok_or(InlineError::InternalError("callee const idx OOB"))?
            .clone();
        caller
            .add_constant(v)
            .map_err(|_| InlineError::InternalError("caller const pool overflow"))
    };
    Ok(match op {
        Op::LoadLocal(i) => InlinedOp::Verbatim(Op::LoadLocal(base + i)),
        Op::StoreLocal(i) => InlinedOp::Verbatim(Op::StoreLocal(base + i)),
        Op::IncLocal(i) => InlinedOp::Verbatim(Op::IncLocal(base + i)),
        Op::Const(k) => InlinedOp::Verbatim(Op::Const(remap_const(k, caller_chunk)?)),
        Op::CallBuiltin { name_const, arity } => InlinedOp::Verbatim(Op::CallBuiltin {
            name_const: remap_const(name_const, caller_chunk)?,
            arity,
        }),
        Op::StructLiteral {
            name_const,
            field_count,
        } => InlinedOp::Verbatim(Op::StructLiteral {
            name_const: remap_const(name_const, caller_chunk)?,
            field_count,
        }),
        Op::GetField { name_const } => InlinedOp::Verbatim(Op::GetField {
            name_const: remap_const(name_const, caller_chunk)?,
        }),
        Op::SetField { name_const } => InlinedOp::Verbatim(Op::SetField {
            name_const: remap_const(name_const, caller_chunk)?,
        }),
        Op::ReturnFromCall => InlinedOp::ReturnAsJump,
        // Jump offsets are relative to PC-after-jump and the inlined
        // body preserves a 1:1 op layout, so internal offsets stay
        // valid; copied verbatim.
        Op::Jump(o) => InlinedOp::Verbatim(Op::Jump(o)),
        Op::JumpIfFalse(o) => InlinedOp::Verbatim(Op::JumpIfFalse(o)),
        Op::JumpIfTrue(o) => InlinedOp::Verbatim(Op::JumpIfTrue(o)),
        // Everything else is a stack op that doesn't reference per-
        // chunk state — copy verbatim.
        other => {
            // Defensive: TailCall / MakeClosure / LoadUpvalue /
            // CallForeign disqualify a function from inlining in
            // is_inlineable, so we should never reach this arm with
            // them. If we do, that's an internal invariant violation.
            match other {
                Op::TailCall(_)
                | Op::MakeClosure { .. }
                | Op::LoadUpvalue(_)
                | Op::CallForeign(_) => {
                    return Err(InlineError::InternalError(
                        "inlineable callee contained disqualifying op",
                    ));
                }
                _ => {}
            }
            InlinedOp::Verbatim(other)
        }
    })
}

// ---------- jump bookkeeping (mirrors peephole.rs / const_fold.rs) ----------

fn jump_target_pc(op: Op, pc: usize) -> Option<usize> {
    let offset = match op {
        Op::Jump(o) | Op::JumpIfFalse(o) | Op::JumpIfTrue(o) => o,
        _ => return None,
    };
    let pc_after = pc as isize + 1;
    let target = pc_after + offset as isize;
    if target < 0 {
        None
    } else {
        Some(target as usize)
    }
}

fn is_jump_op(op: Op) -> bool {
    matches!(op, Op::Jump(_) | Op::JumpIfFalse(_) | Op::JumpIfTrue(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;
    use crate::bytecode::Function;

    fn mk_chunk(code: Vec<Op>, constants: Vec<Value>, lines: Vec<u32>) -> Chunk {
        Chunk {
            code,
            constants,
            line_info: lines,
        }
    }

    /// `fn id(x) { return x; }` — 2-op body: LoadLocal(0); ReturnFromCall.
    /// The `return x;` lowers to LoadLocal(0); ReturnFromCall, plus the
    /// compiler-emitted trailing ReturnFromCall (which body_op_count
    /// already trims).
    fn mk_id_function() -> Function {
        Function {
            name: "id".to_string(),
            arity: 1,
            local_count: 1,
            chunk: mk_chunk(
                vec![Op::LoadLocal(0), Op::ReturnFromCall, Op::ReturnFromCall],
                vec![],
                vec![1, 1, 1],
            ),
        }
    }

    /// `fn add1(x) { return x + 1; }` — 4-op body.
    fn mk_add1_function() -> Function {
        Function {
            name: "add1".to_string(),
            arity: 1,
            local_count: 1,
            chunk: mk_chunk(
                vec![
                    Op::LoadLocal(0),
                    Op::Const(0), // 1
                    Op::Add,
                    Op::ReturnFromCall,
                    Op::ReturnFromCall,
                ],
                vec![Value::Int(1)],
                vec![1, 1, 1, 1, 1],
            ),
        }
    }

    /// A self-recursive function that must NOT inline.
    fn mk_recursive_function() -> Function {
        Function {
            name: "rec".to_string(),
            arity: 0,
            local_count: 0,
            chunk: mk_chunk(vec![Op::Call(0), Op::ReturnFromCall], vec![], vec![1, 1]),
        }
    }

    /// A function with 50 body ops — well over the inline threshold,
    /// must NOT be inlined regardless of how INLINE_THRESHOLD is tuned.
    fn mk_large_function() -> Function {
        let mut code = Vec::new();
        let mut lines = Vec::new();
        // 25 LoadLocal/Const pairs: 50 ops total. Way over threshold.
        for _ in 0..25 {
            code.push(Op::LoadLocal(0));
            code.push(Op::Const(0));
            lines.push(1);
            lines.push(1);
        }
        code.push(Op::ReturnFromCall);
        lines.push(1);
        Function {
            name: "big".to_string(),
            arity: 1,
            local_count: 1,
            chunk: mk_chunk(code, vec![Value::Int(0)], lines),
        }
    }

    fn empty_program(funcs: Vec<Function>) -> Program {
        Program {
            main: Chunk::new(),
            functions: funcs,
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        }
    }

    // ---------- is_inlineable ----------

    #[test]
    fn small_leaf_function_is_inlineable() {
        let funcs = vec![mk_id_function()];
        assert!(is_inlineable(&funcs, 0));
    }

    #[test]
    fn recursive_function_is_not_inlineable() {
        let funcs = vec![mk_recursive_function()];
        assert!(!is_inlineable(&funcs, 0));
    }

    #[test]
    fn large_function_is_not_inlineable() {
        let funcs = vec![mk_large_function()];
        assert!(!is_inlineable(&funcs, 0));
    }

    #[test]
    fn function_at_threshold_is_inlineable() {
        // A function with exactly INLINE_THRESHOLD body ops is on
        // the eligibility boundary — the predicate is `body_len <=
        // INLINE_THRESHOLD`, so it inlines.
        let mut code: Vec<Op> = (0..INLINE_THRESHOLD).map(|_| Op::LoadLocal(0)).collect();
        code.push(Op::ReturnFromCall);
        code.push(Op::ReturnFromCall);
        let lines = vec![1u32; code.len()];
        let func = Function {
            name: "boundary".to_string(),
            arity: 1,
            local_count: 1,
            chunk: mk_chunk(code, vec![], lines),
        };
        let funcs = vec![func];
        assert!(is_inlineable(&funcs, 0));
    }

    #[test]
    fn function_one_over_threshold_is_not_inlineable() {
        // INLINE_THRESHOLD + 1 body ops: just over the eligibility
        // boundary. Must NOT inline.
        let mut code: Vec<Op> = (0..=INLINE_THRESHOLD).map(|_| Op::LoadLocal(0)).collect();
        code.push(Op::ReturnFromCall);
        code.push(Op::ReturnFromCall);
        let lines = vec![1u32; code.len()];
        let func = Function {
            name: "over".to_string(),
            arity: 1,
            local_count: 1,
            chunk: mk_chunk(code, vec![], lines),
        };
        let funcs = vec![func];
        assert!(!is_inlineable(&funcs, 0));
    }

    #[test]
    fn function_with_tailcall_is_not_inlineable() {
        let func = Function {
            name: "tc".to_string(),
            arity: 0,
            local_count: 0,
            chunk: mk_chunk(vec![Op::TailCall(0), Op::Return], vec![], vec![1, 1]),
        };
        let funcs = vec![func];
        assert!(!is_inlineable(&funcs, 0));
    }

    #[test]
    fn function_with_foreign_call_is_not_inlineable() {
        let func = Function {
            name: "ffi_caller".to_string(),
            arity: 0,
            local_count: 0,
            chunk: mk_chunk(
                vec![Op::CallForeign(0), Op::ReturnFromCall],
                vec![],
                vec![1, 1],
            ),
        };
        let funcs = vec![func];
        assert!(!is_inlineable(&funcs, 0));
    }

    #[test]
    fn function_with_closure_op_is_not_inlineable() {
        let func = Function {
            name: "cl".to_string(),
            arity: 0,
            local_count: 0,
            chunk: mk_chunk(
                vec![
                    Op::MakeClosure {
                        fn_idx: 0,
                        upvalue_count: 0,
                    },
                    Op::ReturnFromCall,
                ],
                vec![],
                vec![1, 1],
            ),
        };
        let funcs = vec![func];
        assert!(!is_inlineable(&funcs, 0));
    }

    // ---------- body_op_count ----------

    #[test]
    fn body_op_count_strips_trailing_returnfromcall() {
        // Body of `id` as compiled: LoadLocal(0); ReturnFromCall (from
        // `return x;`); ReturnFromCall (compiler-emitted terminator).
        // body_op_count strips both trailing terminators, leaving the
        // single LoadLocal (count=1).
        let f = mk_id_function();
        assert_eq!(body_op_count(&f.chunk), 1);
    }

    #[test]
    fn body_op_count_strips_one_return_tombstone_too() {
        // After rewrite_tail_calls a chunk may end in `Return;
        // ReturnFromCall`. We strip up to two terminators.
        let chunk = mk_chunk(
            vec![Op::LoadLocal(0), Op::Return, Op::ReturnFromCall],
            vec![],
            vec![1, 1, 1],
        );
        assert_eq!(body_op_count(&chunk), 1);
    }

    // ---------- inline pass — basic shape ----------

    #[test]
    fn inlines_call_to_small_function_into_main() {
        // Main: Const(7); Call(0); Return.
        // After inlining `id`: Const(7); StoreLocal(0); LoadLocal(0); Jump(0); Return.
        // (the Jump-replacement-of-RFC lands on the Return.)
        let id = mk_id_function();
        let mut prog = empty_program(vec![id]);
        prog.main = mk_chunk(
            vec![Op::Const(0), Op::Call(0), Op::Return],
            vec![Value::Int(7)],
            vec![1, 1, 1],
        );

        optimize(&mut prog).unwrap();

        // The Call op must be gone.
        assert!(
            !prog.main.code.iter().any(|op| matches!(op, Op::Call(_))),
            "expected Call to be inlined away: {:?}",
            prog.main.code
        );
        // Some StoreLocal + LoadLocal pair from the inlined body must
        // be present.
        let has_store = prog
            .main
            .code
            .iter()
            .any(|op| matches!(op, Op::StoreLocal(_)));
        let has_load = prog
            .main
            .code
            .iter()
            .any(|op| matches!(op, Op::LoadLocal(_)));
        assert!(
            has_store && has_load,
            "missing inlined body ops: {:?}",
            prog.main.code
        );
    }

    #[test]
    fn does_not_inline_recursive_call() {
        // Main: Call(0); Return. Function 0 is self-recursive.
        let rec = mk_recursive_function();
        let mut prog = empty_program(vec![rec]);
        prog.main = mk_chunk(vec![Op::Call(0), Op::Return], vec![], vec![1, 1]);

        optimize(&mut prog).unwrap();

        // The Call op must still be there.
        assert!(
            prog.main.code.iter().any(|op| matches!(op, Op::Call(0))),
            "Call(0) must survive when callee is recursive: {:?}",
            prog.main.code
        );
    }

    #[test]
    fn does_not_inline_large_function() {
        let big = mk_large_function();
        let mut prog = empty_program(vec![big]);
        prog.main = mk_chunk(
            vec![Op::Const(0), Op::Call(0), Op::Return],
            vec![Value::Int(7)],
            vec![1, 1, 1],
        );

        optimize(&mut prog).unwrap();

        assert!(
            prog.main.code.iter().any(|op| matches!(op, Op::Call(0))),
            "Call(0) must survive when callee exceeds threshold: {:?}",
            prog.main.code
        );
    }

    #[test]
    fn inlines_zero_arg_function() {
        // `fn zero() { return 0; }` — no params, no args to push.
        let zero = Function {
            name: "zero".to_string(),
            arity: 0,
            local_count: 0,
            chunk: mk_chunk(
                vec![Op::Const(0), Op::ReturnFromCall, Op::ReturnFromCall],
                vec![Value::Int(0)],
                vec![1, 1, 1],
            ),
        };
        let mut prog = empty_program(vec![zero]);
        prog.main = mk_chunk(vec![Op::Call(0), Op::Return], vec![], vec![1, 1]);

        optimize(&mut prog).unwrap();

        assert!(
            !prog.main.code.iter().any(|op| matches!(op, Op::Call(_))),
            "Call should be inlined: {:?}",
            prog.main.code
        );
        // The Const for `0` should be re-interned in main's pool.
        assert!(
            prog.main
                .constants
                .iter()
                .any(|v| matches!(v, Value::Int(0))),
            "expected Int(0) in main's constants: {:?}",
            prog.main.constants
        );
    }

    #[test]
    fn inlines_function_body_with_arithmetic_op() {
        // `add1(x) { return x + 1; }` — body has Add. Must survive
        // verbatim in the inlined sequence.
        let add1 = mk_add1_function();
        let mut prog = empty_program(vec![add1]);
        prog.main = mk_chunk(
            vec![Op::Const(0), Op::Call(0), Op::Return],
            vec![Value::Int(7)],
            vec![1, 1, 1],
        );

        optimize(&mut prog).unwrap();

        assert!(
            !prog.main.code.iter().any(|op| matches!(op, Op::Call(_))),
            "Call should be inlined: {:?}",
            prog.main.code
        );
        assert!(
            prog.main.code.iter().any(|op| matches!(op, Op::Add)),
            "Add op from add1 body must appear: {:?}",
            prog.main.code
        );
    }

    #[test]
    fn idempotent_when_no_inlineable_callees() {
        // A program whose only fn is recursive — no inlining possible.
        let rec = mk_recursive_function();
        let mut prog = empty_program(vec![rec]);
        prog.main = mk_chunk(vec![Op::Return], vec![], vec![1]);
        let before = prog.main.code.clone();
        optimize(&mut prog).unwrap();
        assert_eq!(prog.main.code, before);
    }

    #[test]
    fn empty_program_is_ok() {
        let mut prog = empty_program(vec![]);
        prog.main = mk_chunk(vec![Op::Return], vec![], vec![1]);
        optimize(&mut prog).unwrap();
        assert_eq!(prog.main.code.len(), 1);
    }

    // ---------- env-var gating ----------

    /// All env-var-touching tests serialize through this mutex. The
    /// `RESILIENT_INLINE` env var gates the whole inliner pipeline, and
    /// other compiler tests in this binary may run concurrently. We
    /// take the lock for the full duration of any test that mutates
    /// the var, so reads during gating from other tests' calls into
    /// `parse_and_compile` are not interleaved.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn optimize_if_enabled_is_noop_when_var_unset() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let saved = std::env::var("RESILIENT_INLINE").ok();
        // SAFETY: serialized via ENV_LOCK above.
        unsafe {
            std::env::remove_var("RESILIENT_INLINE");
        }
        let id = mk_id_function();
        let mut prog = empty_program(vec![id]);
        prog.main = mk_chunk(
            vec![Op::Const(0), Op::Call(0), Op::Return],
            vec![Value::Int(1)],
            vec![1, 1, 1],
        );

        optimize_if_enabled(&mut prog).unwrap();

        // Call must survive — pass disabled.
        assert!(prog.main.code.iter().any(|op| matches!(op, Op::Call(0))));

        // SAFETY: serialized via ENV_LOCK above.
        unsafe {
            match saved {
                Some(v) => std::env::set_var("RESILIENT_INLINE", v),
                None => std::env::remove_var("RESILIENT_INLINE"),
            }
        }
    }

    // ---------- differential test: env-var gates default behavior ----------

    #[test]
    fn default_off_preserves_call_op_in_compiled_program() {
        // End-to-end: a program with a small leaf fn must compile to
        // a chunk that contains Op::Call when RESILIENT_INLINE is unset
        // (matching the const_fold gating discipline). The pipeline
        // wiring lives in compiler.rs::compile.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let saved = std::env::var("RESILIENT_INLINE").ok();
        // SAFETY: serialized via ENV_LOCK above.
        unsafe {
            std::env::remove_var("RESILIENT_INLINE");
        }
        let prog = crate::compiler::parse_and_compile("fn id(int x) -> int { return x; } id(7);")
            .expect("compiles");
        assert!(
            prog.main.code.iter().any(|op| matches!(op, Op::Call(_))),
            "RESILIENT_INLINE unset: Call must survive: {:?}",
            prog.main.code
        );
        // SAFETY: serialized via ENV_LOCK above.
        unsafe {
            match saved {
                Some(v) => std::env::set_var("RESILIENT_INLINE", v),
                None => std::env::remove_var("RESILIENT_INLINE"),
            }
        }
    }

    /// Differential test: compile the same source with and without
    /// the env var. With the var set, `Call` ops should be eliminated
    /// at every inlineable site; without it, the bytecode shape
    /// matches the un-optimized output. The VALUE of the program (if
    /// it ran) is identical either way — that's the correctness
    /// guarantee inlining must preserve. This test asserts the
    /// SHAPE flip; vm.rs/integration tests cover behavioral
    /// equivalence.
    ///
    /// Tests in a single binary share env vars and run on multiple
    /// threads. We serialize via a global mutex and restore the var
    /// on the way out so other tests see a clean environment.
    #[test]
    fn env_var_toggles_inlining_observably() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        let saved = std::env::var("RESILIENT_INLINE").ok();
        let src = "fn id(int x) -> int { return x; } id(7);";

        // Off: Call must survive.
        // SAFETY: serialized via LOCK above.
        unsafe {
            std::env::remove_var("RESILIENT_INLINE");
        }
        let p_off = crate::compiler::parse_and_compile(src).expect("compiles");
        assert!(
            p_off.main.code.iter().any(|op| matches!(op, Op::Call(_))),
            "off: Call should survive"
        );

        // On: Call should be inlined away.
        // SAFETY: serialized via LOCK above.
        unsafe {
            std::env::set_var("RESILIENT_INLINE", "1");
        }
        let p_on = crate::compiler::parse_and_compile(src).expect("compiles");
        assert!(
            !p_on.main.code.iter().any(|op| matches!(op, Op::Call(_))),
            "on: Call should be inlined away: {:?}",
            p_on.main.code
        );

        // Restore prior value.
        // SAFETY: serialized via LOCK above.
        unsafe {
            match saved {
                Some(v) => std::env::set_var("RESILIENT_INLINE", v),
                None => std::env::remove_var("RESILIENT_INLINE"),
            }
        }
    }
}
