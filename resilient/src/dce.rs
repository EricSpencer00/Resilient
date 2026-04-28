//! RES-297: dead code elimination pass over compiled `Chunk` bytecode.
//!
//! Two rules:
//!
//! 1. **After-return truncation**: after an unconditional terminator
//!    (`Op::Return` or `Op::ReturnFromCall`) in a straight-line basic
//!    block with no forward jumps following it, subsequent ops are
//!    unreachable and are removed.
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
    remove_after_return(chunk);
    fold_constant_branches(chunk);
}

// --------------------------------------------------------------------------
// Rule 1: truncate ops after an unconditional terminator
// --------------------------------------------------------------------------

/// Walk `chunk.code` until the first unconditional terminator (`Return` or
/// `ReturnFromCall`) that is NOT followed by any jump op.  All instructions
/// after that terminator are unreachable and are removed along with their
/// matching `line_info` entries.
///
/// Conservative rule: if *any* jump appears after the first terminator we
/// abort — there might be jump targets in the suffix we cannot prove dead
/// without a full control-flow-graph analysis.
fn remove_after_return(chunk: &mut Chunk) {
    let mut truncate_at: Option<usize> = None;
    for (i, op) in chunk.code.iter().enumerate() {
        match op {
            Op::Return | Op::ReturnFromCall if truncate_at.is_none() => {
                truncate_at = Some(i + 1);
            }
            Op::Return | Op::ReturnFromCall => {}
            Op::Jump(_) | Op::JumpIfFalse(_) | Op::JumpIfTrue(_) => {
                // A jump after the first return could be a forward target;
                // give up and preserve everything.
                return;
            }
            _ => {}
        }
    }
    if let Some(idx) = truncate_at
        && idx < chunk.code.len()
    {
        chunk.code.truncate(idx);
        chunk.line_info.truncate(idx);
    }
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
    fn return_followed_by_jump_is_left_alone() {
        // Conservative: can't truncate when a jump follows the return —
        // the jump might be a target for some predecessor.
        let mut chunk = chunk_from_ops(vec![Op::Return, Op::Jump(0), Op::Add]);
        let original_len = chunk.code.len();
        eliminate(&mut chunk);
        assert_eq!(
            chunk.code.len(),
            original_len,
            "should not truncate when jump follows return"
        );
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
