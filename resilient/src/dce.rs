//! RES-297: dead code elimination pass over compiled `Chunk` bytecode.
//!
//! Two rules:
//!
//! 1. **Reachability-based dead code removal**: compute the set of PCs
//!    reachable from PC 0 via forward fall-through and explicit jumps.
//!    Any PC not in the reachable set is removed with its `line_info`
//!    entry.  Jump offsets are re-linked through an `old_pc → new_pc`
//!    fixup table (same technique as the peephole pass).
//!
//!    This supersedes the old conservative "abort on any jump after the
//!    first return" heuristic, which would leave dead code in place
//!    whenever a conditional branch appeared after an early return inside
//!    an `if` branch.
//!
//! 2. **Constant-branch folding**: when a `Op::Const` loading a
//!    `Value::Bool` immediately precedes a conditional jump, the branch
//!    direction is statically known.  The dead branch and the
//!    now-useless load are removed; jump offsets throughout the chunk
//!    are re-linked via an `old_pc → new_pc` fixup table (same
//!    technique as the peephole pass in `peephole.rs`).

use crate::Value;
use crate::bytecode::{Chunk, Op};

// --------------------------------------------------------------------------
// Public entry point
// --------------------------------------------------------------------------

/// Remove dead code from a compiled chunk in-place.
pub fn eliminate(chunk: &mut Chunk) {
    remove_unreachable(chunk);
    fold_constant_branches(chunk);
}

// --------------------------------------------------------------------------
// Rule 1: reachability-based dead instruction removal
// --------------------------------------------------------------------------

/// Remove instructions that are unreachable from PC 0.
///
/// Computes the reachable set via a BFS/worklist starting at PC 0:
/// - Fall-through: `pc + 1` is reachable from any reachable non-terminator
///   that is not an unconditional `Jump`.
/// - Jump targets: the destination of any reachable jump is reachable.
/// - `Return` / `ReturnFromCall` / unconditional `Jump` do not fall through.
///
/// After computing the reachable set, unreachable instructions are dropped
/// and jump offsets are re-linked via an `old_pc → new_pc` fixup table.
fn remove_unreachable(chunk: &mut Chunk) {
    let n = chunk.code.len();
    if n == 0 {
        return;
    }

    // --- BFS over PCs ---
    let mut reachable = vec![false; n];
    let mut worklist = vec![0usize];
    reachable[0] = true;

    while let Some(pc) = worklist.pop() {
        let op = chunk.code[pc];
        // Compute explicit jump target (if this op is a jump).
        let jump_target = jump_target_pc(op, pc);
        if let Some(t) = jump_target
            && t < n
            && !reachable[t]
        {
            reachable[t] = true;
            worklist.push(t);
        }
        // Fall-through: everything except unconditional terminators.
        let falls_through = !matches!(op, Op::Return | Op::ReturnFromCall | Op::Jump(_));
        let next_pc = pc + 1;
        if falls_through && next_pc < n && !reachable[next_pc] {
            reachable[next_pc] = true;
            worklist.push(next_pc);
        }
    }

    // Fast-exit: nothing to remove.
    if reachable.iter().all(|&r| r) {
        return;
    }

    // --- Build compacted instruction stream ---
    // old_to_new[old_pc] = new_pc (usize::MAX for dropped PCs).
    let mut old_to_new = vec![usize::MAX; n + 1];
    let mut new_code: Vec<Op> = Vec::with_capacity(n);
    let mut new_line_info: Vec<u32> = Vec::with_capacity(n);

    // Capture jump targets from original offsets BEFORE we mutate.
    let orig_targets: Vec<Option<usize>> = chunk
        .code
        .iter()
        .enumerate()
        .map(|(pc, &op)| jump_target_pc(op, pc))
        .collect();

    for (pc, &op) in chunk.code.iter().enumerate() {
        if reachable[pc] {
            old_to_new[pc] = new_code.len();
            new_code.push(op);
            new_line_info.push(chunk.line_info[pc]);
        }
    }
    // Sentinel for end-of-code.
    old_to_new[n] = new_code.len();

    // --- Re-link jump offsets ---
    for (new_pc, op) in new_code.iter_mut().enumerate() {
        if !is_jump_op(*op) {
            continue;
        }
        // Find the old PC that produced this new_pc position.
        let Some(old_pc) = (0..n).find(|&p| old_to_new[p] == new_pc) else {
            continue;
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
    chunk.line_info = new_line_info;
}

// --------------------------------------------------------------------------
// Rule 2: fold constant-condition branches
// --------------------------------------------------------------------------

/// Describes what folding a `Const(bool) + conditional-jump` pair produces.
enum FoldAction {
    /// Both ops vanish — the branch is never taken, just fall through.
    RemoveBoth,
    /// Both ops collapse to an unconditional `Jump`; the caller supplies
    /// the *original absolute target PC* that the conditional jump pointed to.
    ReplaceWithJump(usize),
}

/// Check whether `chunk.code[i..i+2]` is a foldable `Const(bool) + cond-jump`
/// pair.  Returns `None` if the pattern doesn't match or if the *second* op
/// (`code[i+1]`) is a jump target from elsewhere (conservative safety).
fn try_fold_const_branch(chunk: &Chunk, i: usize, targets: &[bool]) -> Option<FoldAction> {
    // The second op must not be a jump target from anywhere else.
    if targets.get(i + 1).copied().unwrap_or(false) {
        return None;
    }

    // First op must be Const loading a Bool.
    let Op::Const(cidx) = chunk.code[i] else {
        return None;
    };
    let bool_val = match chunk.constants.get(cidx as usize) {
        Some(Value::Bool(b)) => *b,
        _ => return None,
    };

    // Second op must be a conditional jump.
    match chunk.code[i + 1] {
        Op::JumpIfFalse(off) => {
            let target_pc = jump_target_pc(Op::JumpIfFalse(off), i + 1)?;
            if bool_val {
                // true + JumpIfFalse → jump never taken → remove both, fall through.
                Some(FoldAction::RemoveBoth)
            } else {
                // false + JumpIfFalse → jump always taken → replace with Jump.
                Some(FoldAction::ReplaceWithJump(target_pc))
            }
        }
        Op::JumpIfTrue(off) => {
            let target_pc = jump_target_pc(Op::JumpIfTrue(off), i + 1)?;
            if bool_val {
                // true + JumpIfTrue → jump always taken → replace with Jump.
                Some(FoldAction::ReplaceWithJump(target_pc))
            } else {
                // false + JumpIfTrue → jump never taken → remove both, fall through.
                Some(FoldAction::RemoveBoth)
            }
        }
        _ => None,
    }
}

/// Scan for `Const(bool) + conditional-jump` pairs and fold them.
///
/// After editing the instruction stream, all `Jump`/`JumpIfFalse`/
/// `JumpIfTrue` offsets are re-linked via an `old_pc → new_pc` table
/// so the relative offsets stay correct.
fn fold_constant_branches(chunk: &mut Chunk) {
    // RES-1415: fast-reject — `try_fold_const_branch` can only return
    // `Some(...)` when position `i` holds an `Op::Const(_)` that maps
    // to a `Value::Bool`. Without any `Op::Const` op in the chunk at
    // all, no fold can fire and the four allocations below
    // (`orig_targets`, `new_code`, `new_lines`, `old_to_new`,
    // `replacement_jumps`) plus the O(jumps × code.len()) jump-fixup
    // post-pass run for no observable change. The const-fold pass
    // already filters its own input, but this DCE pass runs on every
    // chunk regardless, even on those the const-folder reduced to no
    // Op::Const ops (e.g. fully folded leaf arithmetic).
    //
    // Same shape as RES-1407 (peephole skip-fixup) and the
    // `folded_any` early-out in `const_fold::fold_pass`.
    if !chunk.code.iter().any(|op| matches!(op, Op::Const(_))) {
        return;
    }

    // Precompute absolute target PC for each instruction using original offsets.
    let orig_targets: Vec<Option<usize>> = chunk
        .code
        .iter()
        .enumerate()
        .map(|(pc, &op)| jump_target_pc(op, pc))
        .collect();

    // Precompute which PCs are jump targets (conservative guard).
    let targets = jump_targets(chunk);

    let mut new_code: Vec<Op> = Vec::with_capacity(chunk.code.len());
    let mut new_line_info: Vec<u32> = Vec::with_capacity(chunk.code.len());
    // `old_to_new[old_pc]` = corresponding new_pc; usize::MAX means "dropped".
    let mut old_to_new: Vec<usize> = vec![usize::MAX; chunk.code.len() + 1];
    // For folded Const+cond-jump → Jump replacements: (new_pc, orig_abs_target).
    let mut replacement_jumps: Vec<(usize, usize)> = Vec::new();
    // RES-1415: track whether any fold fired so the jump-fixup pass
    // (O(jumps × code.len()) per the post-pass linear scan below)
    // can be skipped when no Const+cond-jump pair matched. Mirrors
    // the `optimized_any` early-out in `peephole::optimize` (RES-1407).
    let mut folded_any = false;

    let mut i = 0;
    while i < chunk.code.len() {
        old_to_new[i] = new_code.len();

        if i + 1 < chunk.code.len()
            && let Some(action) = try_fold_const_branch(chunk, i, &targets)
        {
            match action {
                FoldAction::RemoveBoth => {
                    // Both ops dropped; the second old_pc maps to the next slot.
                    old_to_new[i + 1] = new_code.len();
                    folded_any = true;
                    i += 2;
                    continue;
                }
                FoldAction::ReplaceWithJump(old_target_pc) => {
                    // Replace the pair with a single unconditional Jump.
                    // Use offset 0 as a placeholder; the re-link pass will fix it.
                    let new_pc = new_code.len();
                    new_code.push(Op::Jump(0));
                    new_line_info.push(chunk.line_info[i]);
                    replacement_jumps.push((new_pc, old_target_pc));
                    old_to_new[i + 1] = new_code.len(); // second op is dropped
                    folded_any = true;
                    i += 2;
                    continue;
                }
            }
        }

        // No fold — copy verbatim.
        new_code.push(chunk.code[i]);
        new_line_info.push(chunk.line_info[i]);
        i += 1;
    }
    // Sentinel: end-of-code target.
    old_to_new[chunk.code.len()] = new_code.len();

    // RES-1415: skip the relink pass + Vec swap when no fold fired.
    // `new_code` is byte-identical to `chunk.code` in that case and
    // every `old_to_new[i] == i`, so the jump fixup would recompute
    // offsets that already equal themselves. Same pattern as
    // `peephole::optimize`'s `optimized_any` early-out (RES-1407).
    if !folded_any {
        return;
    }

    // Re-link pass for all surviving jump ops (not the replacement Jumps,
    // which are handled separately below).
    for (new_pc, op) in new_code.iter_mut().enumerate() {
        if !is_jump_op(*op) {
            continue;
        }
        // Skip replacement Jump placeholders — we fix those below.
        if replacement_jumps.iter().any(|(rp, _)| *rp == new_pc) {
            continue;
        }
        // Find the originating old PC by scanning old_to_new.
        let Some(old_pc) = (0..chunk.code.len()).find(|&p| old_to_new[p] == new_pc) else {
            continue;
        };
        let Some(old_target) = orig_targets[old_pc] else {
            continue;
        };
        relink_jump(op, new_pc, old_target, &old_to_new);
    }

    // Fix replacement Jump offsets using the stored original target PCs.
    for (new_pc, old_target_pc) in replacement_jumps {
        if let Some(op) = new_code.get_mut(new_pc) {
            relink_jump(op, new_pc, old_target_pc, &old_to_new);
        }
    }

    chunk.code = new_code;
    chunk.line_info = new_line_info;
}

/// Update the offset of a jump op at `new_pc` so it targets
/// `old_to_new[old_target_pc]`.  No-ops if the mapping is unavailable
/// or the offset overflows `i16`.
fn relink_jump(op: &mut Op, new_pc: usize, old_target_pc: usize, old_to_new: &[usize]) {
    let new_target = match old_to_new.get(old_target_pc) {
        Some(&t) if t != usize::MAX => t,
        _ => return,
    };
    let offset = (new_target as isize) - (new_pc as isize + 1);
    let Ok(off_i16) = i16::try_from(offset) else {
        return;
    };
    match op {
        Op::Jump(o) => *o = off_i16,
        Op::JumpIfFalse(o) => *o = off_i16,
        Op::JumpIfTrue(o) => *o = off_i16,
        _ => {}
    }
}

// --------------------------------------------------------------------------
// Shared helpers (mirrors peephole.rs — kept local to avoid coupling)
// --------------------------------------------------------------------------

/// Return the absolute target PC of a jump instruction at `pc`,
/// or `None` for non-jump ops.
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

/// Compute a bitset (as `Vec<bool>`) of which PCs are the destination of any
/// jump in the chunk.
fn jump_targets(chunk: &Chunk) -> Vec<bool> {
    let n = chunk.code.len();
    let mut out = vec![false; n + 1];
    for (pc, &op) in chunk.code.iter().enumerate() {
        if let Some(t) = jump_target_pc(op, pc)
            && t <= n
        {
            out[t] = true;
        }
    }
    out
}

fn is_jump_op(op: Op) -> bool {
    matches!(op, Op::Jump(_) | Op::JumpIfFalse(_) | Op::JumpIfTrue(_))
}

// --------------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{Chunk, Op};

    fn chunk_from_ops(ops: Vec<Op>) -> Chunk {
        let mut c = Chunk::new();
        for op in ops {
            c.emit(op, 1);
        }
        c
    }

    // ---- Rule 1: after-return truncation ----

    #[test]
    fn ops_after_return_are_removed() {
        let mut chunk = chunk_from_ops(vec![
            Op::Return,
            Op::Add, // dead
            Op::Neg, // dead
        ]);
        eliminate(&mut chunk);
        assert_eq!(chunk.code.len(), 1, "expected 1 op, got {:?}", chunk.code);
        assert_eq!(chunk.code[0], Op::Return);
    }

    #[test]
    fn ops_after_return_from_call_are_removed() {
        let mut chunk = chunk_from_ops(vec![
            Op::ReturnFromCall,
            Op::Add, // dead
        ]);
        eliminate(&mut chunk);
        assert_eq!(chunk.code.len(), 1);
        assert_eq!(chunk.code[0], Op::ReturnFromCall);
    }

    #[test]
    fn return_with_no_trailing_ops_unchanged() {
        let ops = vec![Op::Add, Op::Return];
        let mut chunk = chunk_from_ops(ops.clone());
        eliminate(&mut chunk);
        assert_eq!(chunk.code, ops);
    }

    #[test]
    fn return_followed_by_unreachable_jump_is_removed() {
        // Reachability analysis: Return at PC 0 doesn't fall through; the
        // Jump and Add at PCs 1 and 2 have no predecessor that reaches them,
        // so they are correctly identified as dead and removed.
        let mut chunk = chunk_from_ops(vec![Op::Return, Op::Jump(0), Op::Add]);
        eliminate(&mut chunk);
        assert_eq!(chunk.code.len(), 1, "unreachable ops should be removed: {:?}", chunk.code);
        assert_eq!(chunk.code[0], Op::Return);
    }

    #[test]
    fn post_return_code_kept_when_it_has_predecessor_jump() {
        // [0] JumpIfFalse → targets PC 2 (= Add)
        // [1] Return       (reachable via fall-through when condition is true)
        // [2] Add          (reachable via jump from [0] when condition is false)
        // [3] Return
        let mut chunk = Chunk::new();
        // JumpIfFalse with offset +1: target = (1+1) + 1 = 3 → but we want to
        // jump to PC 2 (Add). offset = 2 - (0+1) = 1.
        chunk.emit(Op::JumpIfFalse(1), 1); // [0] → targets PC 2
        chunk.emit(Op::Return, 1); // [1]
        chunk.emit(Op::Add, 1); // [2] reachable (jump from [0])
        chunk.emit(Op::Return, 1); // [3] reachable (fall-through from [2])
        eliminate(&mut chunk);
        // All four ops are reachable — none should be removed.
        assert_eq!(chunk.code.len(), 4, "all ops reachable: {:?}", chunk.code);
    }

    #[test]
    fn line_info_stays_in_sync_after_truncation() {
        let mut chunk = Chunk::new();
        chunk.emit(Op::Return, 10);
        chunk.emit(Op::Add, 11); // dead
        chunk.emit(Op::Neg, 12); // dead
        eliminate(&mut chunk);
        assert_eq!(chunk.code.len(), 1);
        assert_eq!(chunk.line_info.len(), 1);
        assert_eq!(chunk.line_info[0], 10);
    }

    #[test]
    fn dead_code_after_early_return_inside_conditional_is_removed() {
        // Simulates: if cond { return; } <dead-code>
        // [0] JumpIfFalse → targets PC 2 (else branch)
        // [1] Return        (early return inside if-true branch)
        // [2] Neg           (dead — only reachable after [1] which returns, AND
        //                   the JumpIfFalse target is 2 so it IS reachable via
        //                   the false-branch. This must remain.)
        // [3] Return
        //
        // Actually all 4 are reachable here. Let's test the simpler case:
        // [0] JumpIfFalse → targets PC 3 (skip two dead ops after true-return)
        // [1] Return        (true branch return)
        // [2] Neg           (dead — nothing jumps here, [1] doesn't fall through)
        // [3] Mul           (dead — nothing jumps here, [2] doesn't fall through,
        //                    wait — [0] jumps to PC 3, so Mul IS reachable!)
        // Actually this is getting complex. Simple case: jump skips one dead op.
        //
        // [0] JumpIfFalse(+2) → targets PC 3
        // [1] Return
        // [2] Neg              ← dead (no predecessor reaches it)
        // [3] Mul
        // [4] Return
        let mut chunk = Chunk::new();
        chunk.emit(Op::JumpIfFalse(2), 1); // [0] → PC 3
        chunk.emit(Op::Return, 1);         // [1]
        chunk.emit(Op::Neg, 1);            // [2] dead
        chunk.emit(Op::Mul, 1);            // [3] reachable (jump from [0])
        chunk.emit(Op::Return, 1);         // [4] reachable (fall-through from [3])
        eliminate(&mut chunk);
        // After removing PC 2 (Neg): [JumpIfFalse, Return, Mul, Return]
        assert_eq!(chunk.code.len(), 4, "code: {:?}", chunk.code);
        assert!(matches!(chunk.code[0], Op::JumpIfFalse(_)));
        assert_eq!(chunk.code[1], Op::Return);
        assert_eq!(chunk.code[2], Op::Mul);
        assert_eq!(chunk.code[3], Op::Return);
        // Jump at new pc=0 should now target new pc=2 (Mul):
        // new_target = 2, offset = 2 - (0+1) = 1.
        if let Op::JumpIfFalse(off) = chunk.code[0] {
            assert_eq!(1 + off as isize, 2, "JumpIfFalse should target Mul at index 2");
        }
    }

    // ---- Rule 2: constant-branch folding ----

    #[test]
    fn true_jump_if_false_removes_both() {
        // Bool(true) + JumpIfFalse → jump never taken → remove both.
        let mut chunk = Chunk::new();
        let tidx = chunk.add_constant(Value::Bool(true)).unwrap();
        chunk.emit(Op::Const(tidx), 1); // [0]
        chunk.emit(Op::JumpIfFalse(1), 1); // [1] offset +1 → would skip [2]
        chunk.emit(Op::Add, 1); // [2] target of the JumpIfFalse
        chunk.emit(Op::Return, 1); // [3]
        eliminate(&mut chunk);
        // After removing ops 0+1, we have [Add, Return].
        assert_eq!(chunk.code.len(), 2, "got {:?}", chunk.code);
        assert_eq!(chunk.code[0], Op::Add);
        assert_eq!(chunk.code[1], Op::Return);
    }

    #[test]
    fn false_jump_if_false_becomes_unconditional_jump() {
        // Bool(false) + JumpIfFalse(off) → jump always taken → becomes Jump.
        // Layout: [0] Const(false), [1] JumpIfFalse→[3], [2] Add, [3] Return
        let mut chunk = Chunk::new();
        let fidx = chunk.add_constant(Value::Bool(false)).unwrap();
        chunk.emit(Op::Const(fidx), 1); // [0]
        // JumpIfFalse at [1], offset = +1, targets pc 1+1+1 = 3
        chunk.emit(Op::JumpIfFalse(1), 1); // [1]
        chunk.emit(Op::Add, 1); // [2] (dead branch)
        chunk.emit(Op::Return, 1); // [3]
        eliminate(&mut chunk);
        // Pair [0,1] becomes single Jump; [2] is now reachable but DCE rule
        // 1 is conservative. Result: [Jump(?), Add, Return].
        // The Jump should target [3] which is now new-pc 2 (after the fold).
        // new layout: [0]=Jump, [1]=Add, [2]=Return
        // Jump at new_pc=0 should target new_pc=2 → offset = 2 - (0+1) = 1
        assert_eq!(chunk.code.len(), 3, "got {:?}", chunk.code);
        assert!(
            matches!(chunk.code[0], Op::Jump(_)),
            "expected Jump, got {:?}",
            chunk.code[0]
        );
        if let Op::Jump(off) = chunk.code[0] {
            let target = 1 + off as isize; // pc_after + offset
            assert_eq!(target, 2, "Jump should target index 2, got {}", target);
        }
    }

    #[test]
    fn true_jump_if_true_becomes_unconditional_jump() {
        // Bool(true) + JumpIfTrue → jump always taken.
        let mut chunk = Chunk::new();
        let tidx = chunk.add_constant(Value::Bool(true)).unwrap();
        chunk.emit(Op::Const(tidx), 1); // [0]
        // JumpIfTrue at [1], offset = +1, targets pc 3
        chunk.emit(Op::JumpIfTrue(1), 1); // [1]
        chunk.emit(Op::Add, 1); // [2] dead
        chunk.emit(Op::Return, 1); // [3]
        eliminate(&mut chunk);
        // new: [Jump, Add, Return]; Jump at new_pc=0 targets new_pc=2
        assert_eq!(chunk.code.len(), 3, "got {:?}", chunk.code);
        assert!(matches!(chunk.code[0], Op::Jump(_)));
        if let Op::Jump(off) = chunk.code[0] {
            assert_eq!(1 + off as isize, 2, "jump should target index 2");
        }
    }

    #[test]
    fn false_jump_if_true_removes_both() {
        // Bool(false) + JumpIfTrue → jump never taken → remove both.
        let mut chunk = Chunk::new();
        let fidx = chunk.add_constant(Value::Bool(false)).unwrap();
        chunk.emit(Op::Const(fidx), 1); // [0]
        chunk.emit(Op::JumpIfTrue(1), 1); // [1] would skip to [3]
        chunk.emit(Op::Add, 1); // [2]
        chunk.emit(Op::Return, 1); // [3]
        eliminate(&mut chunk);
        assert_eq!(chunk.code.len(), 2, "got {:?}", chunk.code);
        assert_eq!(chunk.code[0], Op::Add);
        assert_eq!(chunk.code[1], Op::Return);
    }

    #[test]
    fn non_bool_const_is_not_folded() {
        let mut chunk = Chunk::new();
        let idx = chunk.add_constant(Value::Int(1)).unwrap();
        chunk.emit(Op::Const(idx), 1);
        chunk.emit(Op::JumpIfFalse(0), 1);
        chunk.emit(Op::Return, 1);
        let original_len = chunk.code.len();
        eliminate(&mut chunk);
        assert_eq!(
            chunk.code.len(),
            original_len,
            "int const should not be folded"
        );
    }
}
