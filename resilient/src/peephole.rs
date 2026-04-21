//! RES-172: VM peephole optimizer.
//!
//! A single linear-scan pass over `Chunk::code` that folds common
//! bytecode idioms into shorter equivalents. Runs once per chunk,
//! after the compiler finishes emitting. Each rule is behind its
//! own predicate + rewrite function so unit tests can exercise
//! one at a time without the full pass.
//!
//! Rules shipped in this revision:
//!
//! 1. `Const(k==0); Add`                    → drop both (identity)
//! 2. `LoadLocal x; Const(k==1); Add; StoreLocal x`
//!    → `IncLocal(x)`
//! 3. `Jump(0)`                             → drop (fall-through)
//! 4. `Not; JumpIfFalse(off)`               → `JumpIfTrue(off)`
//! 5. `Const(k==1); Mul`                    → drop both (×1 identity)
//! 6. `Const(k==0); Mul`                    → drop both + push Const(0)
//!    (only when preceding op is a pure load: LoadLocal or Const)
//!
//! Strength-reduction rules for power-of-two constants (Mul→Shl,
//! Div→Shr, Mod→BitAnd) are deferred: the opcodes Shl, Shr, and
//! BitAnd do not yet exist in bytecode.rs.
//!
//! Jump relinking: offsets in `Jump` / `JumpIfFalse` / `JumpIfTrue`
//! are relative to the PC *after* the jump, so any rewrite that
//! changes instruction count invalidates every jump that crosses
//! the edit. We handle this via a `old_pc → new_pc` fixup table:
//! compute target PCs from the original offsets up-front, do the
//! rewrite building the map, then rewrite every jump's offset
//! from the map at the end. The ticket calls this out
//! explicitly — no hand-computed offset bookkeeping.
//!
//! Line-info preservation (RES-091): `Chunk::line_info` parallels
//! `code` and is mutated lock-step — when the pass drops
//! instructions, their line entries are dropped too; replacements
//! inherit the line of the original's first instruction. Runtime
//! errors after optimization still blame the correct source line.
//!
//! Jump-target safety: if any interior instruction of a pattern is
//! a jump target from somewhere else in the chunk, the rule is
//! SKIPPED for that site. A collapsed PC range would leave the
//! jump landing nowhere valid; skipping preserves correctness
//! even if it forgoes some optimization opportunities. A future
//! iterative pass could relax this at the cost of analysis.

use crate::Value;
use crate::bytecode::{Chunk, Op};

/// Errors that the peephole optimizer can return.
#[derive(Debug)]
pub enum OptimizeError {
    InternalError(&'static str),
}

impl std::fmt::Display for OptimizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OptimizeError::InternalError(msg) => write!(f, "peephole optimizer: {}", msg),
        }
    }
}

impl std::error::Error for OptimizeError {}

/// Top-level entry. Applies all peephole rules in one linear scan.
/// Idempotent for the rules shipped today (no rule creates a new
/// opportunity the same pass could re-fold).
pub fn optimize(chunk: &mut Chunk) -> Result<(), OptimizeError> {
    // Precompute the set of jump-target PCs so we can skip any
    // rule site whose interior is a jump destination.
    let targets = jump_targets(chunk);

    // Capture each jump's ORIGINAL target PC before we mutate
    // anything; we'll map these through the fixup table at the
    // end to reconstruct offsets in the new layout.
    let orig_targets: Vec<Option<usize>> = chunk
        .code
        .iter()
        .enumerate()
        .map(|(pc, op)| jump_target_pc(*op, pc))
        .collect();

    // Rewrite pass. Build `new_code` + `new_line_info` + an
    // `old_pc → new_pc` map. Dropped instructions map to the
    // next surviving instruction's new PC.
    let mut new_code: Vec<Op> = Vec::with_capacity(chunk.code.len());
    let mut new_line_info: Vec<u32> = Vec::with_capacity(chunk.code.len());
    let mut old_to_new: Vec<usize> = vec![usize::MAX; chunk.code.len() + 1];
    let mut i = 0;
    while i < chunk.code.len() {
        // Record the mapping for the START of each rule window
        // BEFORE we emit anything for it. This way a jump that
        // targets the first op of a fold still lands on the
        // fold's replacement.
        old_to_new[i] = new_code.len();

        // Rule 1 — drop `Const(k==0); Add`. Skip if `Add` is a
        // jump target (a jump into the middle of the pattern).
        if rule_add_zero_identity(chunk, i, &targets) {
            i += 2;
            continue;
        }
        // Rule 2 — fold `LoadLocal x; Const(k==1); Add; StoreLocal x`
        // → `IncLocal(x)`.
        if let Some(idx) = rule_inc_local(chunk, i, &targets) {
            new_code.push(Op::IncLocal(idx));
            new_line_info.push(chunk.line_info[i]);
            i += 4;
            continue;
        }
        // Rule 3 — drop `Jump(0)` (fall-through).
        if rule_dead_jump(chunk, i) {
            i += 1;
            continue;
        }
        // Rule 4 — fold `Not; JumpIfFalse(off)` → `JumpIfTrue(off)`.
        if let Some(off) = rule_not_jif_to_jit(chunk, i, &targets) {
            new_code.push(Op::JumpIfTrue(off));
            new_line_info.push(chunk.line_info[i]);
            i += 2;
            continue;
        }
        // Rule 5 — drop `Const(k==1); Mul` (×1 identity).
        if rule_mul_one_identity(chunk, i, &targets) {
            i += 2;
            continue;
        }
        // Rule 6 — `Const(k==0); Mul` → `Const(0)` when the preceding
        // load is pure (LoadLocal or Const). Replaces three ops with one.
        if rule_mul_zero(chunk, i, &targets, &new_code) {
            // Pop the preceding pure load from the output we already emitted.
            new_code.pop();
            new_line_info.pop();
            // Push Const(0) — reuse the same constant-pool index we already
            // have in the pattern (the zero constant at chunk.code[i]).
            let Op::Const(zero_k) = chunk.code[i] else {
                unreachable!()
            };
            new_code.push(Op::Const(zero_k));
            new_line_info.push(chunk.line_info[i]);
            i += 2;
            continue;
        }

        // No rule fired — copy the instruction verbatim.
        new_code.push(chunk.code[i]);
        new_line_info.push(chunk.line_info[i]);
        i += 1;
    }
    // Sentinel for "end of code" target (fall-off-end PC).
    old_to_new[chunk.code.len()] = new_code.len();

    // Re-link jump offsets. For each JUMP op in new_code, look up
    // which old PC it originated from (scan old_to_new), fetch
    // that old op's original target, map through old_to_new, and
    // compute the new offset.
    //
    // Scanning old_to_new to find the originating old PC per new
    // op is O(n²) worst-case. For the chunk sizes we see today
    // (hundreds of ops max) that's irrelevant; a future pass can
    // carry old_pc alongside each emitted new op if it ever
    // matters. Keep the simple version here.
    for (new_pc, op) in new_code.iter_mut().enumerate() {
        // Only recompute for jump-carrying ops.
        if !is_jump_op(*op) {
            continue;
        }
        // Find the old PC that maps to this new PC. The
        // rewriting loop only inserts one new op per old
        // position (never reorders), so the first old_pc with
        // `old_to_new[old_pc] == new_pc` is the right one.
        let Some(old_pc) = (0..chunk.code.len()).find(|&p| old_to_new[p] == new_pc) else {
            return Err(OptimizeError::InternalError(
                "peephole: new_pc with no originating old_pc",
            ));
        };
        let Some(old_target) = orig_targets[old_pc] else {
            continue; // not actually a jump (shouldn't happen)
        };
        let new_target = old_to_new[old_target];
        // Compute offset relative to PC *after* the jump.
        let offset = (new_target as isize) - (new_pc as isize + 1);
        // Clamp: all offsets in realistic programs fit in i16. If
        // the peephole somehow produced a larger jump (can only
        // happen if the original exceeded i16 — which the compiler
        // would already have rejected), leave the op alone and
        // let downstream error handling deal with it.
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
    Ok(())
}

/// Compute which PCs are the target of any jump in the chunk.
/// Used to skip rule application when an interior pattern
/// position is reachable via a branch — collapsing it would
/// strand the jump.
fn jump_targets(chunk: &Chunk) -> Vec<bool> {
    let n = chunk.code.len();
    let mut out = vec![false; n + 1];
    for (pc, op) in chunk.code.iter().enumerate() {
        if let Some(t) = jump_target_pc(*op, pc)
            && t <= n
        {
            out[t] = true;
        }
    }
    out
}

/// Extract the destination PC of a jump instruction at `pc`,
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

fn is_jump_op(op: Op) -> bool {
    matches!(op, Op::Jump(_) | Op::JumpIfFalse(_) | Op::JumpIfTrue(_))
}

// ---------- individual rule predicates ----------

/// Rule 1: drop `Const(k); Add` when constants[k] is Int(0).
/// Skips if PC i+1 is a jump target.
pub(crate) fn rule_add_zero_identity(chunk: &Chunk, i: usize, targets: &[bool]) -> bool {
    if i + 1 >= chunk.code.len() {
        return false;
    }
    let Op::Const(k) = chunk.code[i] else {
        return false;
    };
    if !matches!(chunk.code[i + 1], Op::Add) {
        return false;
    }
    if !matches!(chunk.constants.get(k as usize), Some(Value::Int(0))) {
        return false;
    }
    // Interior position (i+1) — if anything jumps to Add, the
    // fold would strand the jump.
    if *targets.get(i + 1).unwrap_or(&false) {
        return false;
    }
    true
}

/// Rule 2: fold `LoadLocal x; Const(k==1); Add; StoreLocal x` →
/// `IncLocal(x)`. Returns `Some(x)` on a match.
pub(crate) fn rule_inc_local(chunk: &Chunk, i: usize, targets: &[bool]) -> Option<u16> {
    if i + 3 >= chunk.code.len() {
        return None;
    }
    let Op::LoadLocal(x1) = chunk.code[i] else {
        return None;
    };
    let Op::Const(k) = chunk.code[i + 1] else {
        return None;
    };
    if !matches!(chunk.code[i + 2], Op::Add) {
        return None;
    }
    let Op::StoreLocal(x2) = chunk.code[i + 3] else {
        return None;
    };
    if x1 != x2 {
        return None;
    }
    if !matches!(chunk.constants.get(k as usize), Some(Value::Int(1))) {
        return None;
    }
    // Skip if any interior op is a jump target.
    for j in (i + 1)..=(i + 3) {
        if *targets.get(j).unwrap_or(&false) {
            return None;
        }
    }
    Some(x1)
}

/// Rule 3: drop `Jump(0)` — falls through to the next instruction
/// anyway.
pub(crate) fn rule_dead_jump(chunk: &Chunk, i: usize) -> bool {
    matches!(chunk.code[i], Op::Jump(0))
}

/// Rule 4: fold `Not; JumpIfFalse(off)` → `JumpIfTrue(off)`.
/// Returns `Some(off)` on a match.
pub(crate) fn rule_not_jif_to_jit(chunk: &Chunk, i: usize, targets: &[bool]) -> Option<i16> {
    if i + 1 >= chunk.code.len() {
        return None;
    }
    if !matches!(chunk.code[i], Op::Not) {
        return None;
    }
    let Op::JumpIfFalse(off) = chunk.code[i + 1] else {
        return None;
    };
    if *targets.get(i + 1).unwrap_or(&false) {
        return None;
    }
    Some(off)
}

/// Rule 5: drop `Const(k==1); Mul` — multiplying by one is a no-op.
/// Skips if `Mul` is a jump target.
pub(crate) fn rule_mul_one_identity(chunk: &Chunk, i: usize, targets: &[bool]) -> bool {
    if i + 1 >= chunk.code.len() {
        return false;
    }
    let Op::Const(k) = chunk.code[i] else {
        return false;
    };
    if !matches!(chunk.code[i + 1], Op::Mul) {
        return false;
    }
    if !matches!(chunk.constants.get(k as usize), Some(Value::Int(1))) {
        return false;
    }
    if *targets.get(i + 1).unwrap_or(&false) {
        return false;
    }
    true
}

/// Rule 6: fold `<pure-load>; Const(k==0); Mul` → `Const(0)`.
///
/// A "pure load" is any op that pushes exactly one value onto the stack
/// without side-effects: `LoadLocal(_)` or `Const(_)`. If the op at
/// `i-1` in the already-emitted new_code (i.e., `new_code.last()`) is
/// such a load, AND the window `Const(0); Mul` is at `[i, i+1]`, we
/// can replace all three ops with a single `Const(0)`.
///
/// Skips if `Mul` (at `i+1`) is a jump target.
pub(crate) fn rule_mul_zero(chunk: &Chunk, i: usize, targets: &[bool], new_code: &[Op]) -> bool {
    if i + 1 >= chunk.code.len() {
        return false;
    }
    let Op::Const(k) = chunk.code[i] else {
        return false;
    };
    if !matches!(chunk.code[i + 1], Op::Mul) {
        return false;
    }
    if !matches!(chunk.constants.get(k as usize), Some(Value::Int(0))) {
        return false;
    }
    if *targets.get(i + 1).unwrap_or(&false) {
        return false;
    }
    // The preceding emitted op must be a pure (side-effect-free) load.
    matches!(new_code.last(), Some(Op::LoadLocal(_)) | Some(Op::Const(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Op;

    fn mk_chunk(code: &[Op], constants: Vec<Value>, lines: &[u32]) -> Chunk {
        Chunk {
            code: code.to_vec(),
            constants,
            line_info: lines.to_vec(),
        }
    }

    // ---------- Rule 1: Const(0); Add ----------

    #[test]
    fn rule1_fires_on_const_zero_plus_add() {
        let chunk = mk_chunk(&[Op::Const(0), Op::Add], vec![Value::Int(0)], &[1, 1]);
        assert!(rule_add_zero_identity(&chunk, 0, &[false; 3]));
    }

    #[test]
    fn rule1_skips_when_const_is_nonzero() {
        let chunk = mk_chunk(&[Op::Const(0), Op::Add], vec![Value::Int(5)], &[1, 1]);
        assert!(!rule_add_zero_identity(&chunk, 0, &[false; 3]));
    }

    #[test]
    fn rule1_skips_when_add_is_jump_target() {
        let chunk = mk_chunk(&[Op::Const(0), Op::Add], vec![Value::Int(0)], &[1, 1]);
        let mut targets = vec![false; 3];
        targets[1] = true;
        assert!(!rule_add_zero_identity(&chunk, 0, &targets));
    }

    #[test]
    fn rule1_drops_identity_in_full_pass() {
        // LoadLocal(0) + Const(0) + Add + StoreLocal(0) is the
        // pathological shape the rule targets. After the pass:
        // LoadLocal(0) + StoreLocal(0).
        let mut chunk = mk_chunk(
            &[Op::LoadLocal(0), Op::Const(0), Op::Add, Op::StoreLocal(0)],
            vec![Value::Int(0)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code, vec![Op::LoadLocal(0), Op::StoreLocal(0)]);
        assert_eq!(chunk.line_info, vec![1, 1]);
    }

    // ---------- Rule 2: IncLocal fold ----------

    #[test]
    fn rule2_fires_on_inc_idiom() {
        let chunk = mk_chunk(
            &[Op::LoadLocal(3), Op::Const(0), Op::Add, Op::StoreLocal(3)],
            vec![Value::Int(1)],
            &[1, 1, 1, 1],
        );
        assert_eq!(rule_inc_local(&chunk, 0, &[false; 5]), Some(3));
    }

    #[test]
    fn rule2_skips_mismatched_locals() {
        let chunk = mk_chunk(
            &[Op::LoadLocal(3), Op::Const(0), Op::Add, Op::StoreLocal(4)],
            vec![Value::Int(1)],
            &[1, 1, 1, 1],
        );
        assert_eq!(rule_inc_local(&chunk, 0, &[false; 5]), None);
    }

    #[test]
    fn rule2_skips_non_one_constant() {
        let chunk = mk_chunk(
            &[Op::LoadLocal(3), Op::Const(0), Op::Add, Op::StoreLocal(3)],
            vec![Value::Int(5)],
            &[1, 1, 1, 1],
        );
        assert_eq!(rule_inc_local(&chunk, 0, &[false; 5]), None);
    }

    #[test]
    fn rule2_folds_in_full_pass() {
        let mut chunk = mk_chunk(
            &[
                Op::LoadLocal(2),
                Op::Const(0),
                Op::Add,
                Op::StoreLocal(2),
                Op::Return,
            ],
            vec![Value::Int(1)],
            &[7, 7, 7, 7, 8],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code, vec![Op::IncLocal(2), Op::Return]);
        // Line info: first inst of the fold's line, then the
        // Return's unchanged line.
        assert_eq!(chunk.line_info, vec![7, 8]);
    }

    // ---------- Rule 3: Jump(0) drop ----------

    #[test]
    fn rule3_fires_on_zero_jump() {
        let chunk = mk_chunk(&[Op::Jump(0)], vec![], &[1]);
        assert!(rule_dead_jump(&chunk, 0));
    }

    #[test]
    fn rule3_skips_nonzero_jump() {
        let chunk = mk_chunk(&[Op::Jump(2)], vec![], &[1]);
        assert!(!rule_dead_jump(&chunk, 0));
    }

    #[test]
    fn rule3_drops_dead_jump_in_full_pass() {
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Jump(0), Op::Return],
            vec![Value::Int(42)],
            &[1, 1, 2],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code, vec![Op::Const(0), Op::Return]);
    }

    // ---------- Rule 4: Not; JumpIfFalse → JumpIfTrue ----------

    #[test]
    fn rule4_fires_on_not_jif() {
        let chunk = mk_chunk(&[Op::Not, Op::JumpIfFalse(5)], vec![], &[1, 1]);
        assert_eq!(rule_not_jif_to_jit(&chunk, 0, &[false; 3]), Some(5));
    }

    #[test]
    fn rule4_folds_in_full_pass() {
        // Old layout (4 ops):
        //   0: Not
        //   1: JumpIfFalse(1)        → target = 1+1+1 = 3 (Return)
        //   2: Const(0)
        //   3: Return
        //
        // After fold:
        //   0: JumpIfTrue(?)         → target = old 3 (Return) in
        //                              new layout
        //   1: Const(0)
        //   2: Return
        //
        // Mapping: old 0,1 → new 0; old 2 → new 1; old 3 → new 2.
        // So JumpIfTrue's new offset = new_target(2) - (0+1) = 1.
        let mut chunk = mk_chunk(
            &[Op::Not, Op::JumpIfFalse(1), Op::Const(0), Op::Return],
            vec![Value::Int(1)],
            &[1, 1, 2, 3],
        );
        optimize(&mut chunk).unwrap();
        match chunk.code[0] {
            Op::JumpIfTrue(o) => assert_eq!(o, 1),
            other => panic!("expected JumpIfTrue, got {:?}", other),
        }
        assert_eq!(chunk.code.len(), 3);
    }

    // ---------- Jump-target safety ----------

    #[test]
    fn rule_skipped_when_interior_is_jump_target() {
        // A Jump(1) at PC=0 lands on PC=2, which is the Add inside
        // a `Const(0); Add` pattern starting at PC=1. The fold
        // must NOT fire — it would strand the jump target.
        let mut chunk = mk_chunk(
            &[Op::Jump(1), Op::Const(0), Op::Add, Op::Return],
            vec![Value::Int(0)],
            &[1, 1, 1, 2],
        );
        optimize(&mut chunk).unwrap();
        // Code length unchanged: the peephole skipped the fold.
        assert_eq!(chunk.code.len(), 4);
        assert!(matches!(chunk.code[1], Op::Const(_)));
        assert!(matches!(chunk.code[2], Op::Add));
    }

    // ---------- Relink correctness ----------

    #[test]
    fn jumps_relink_across_dropped_instructions() {
        // Build a chunk where a forward JumpIfFalse skips over a
        // `Const(0); Add` that the peephole will fold away. The
        // jump's target must still land on the right instruction
        // after the fold.
        //
        // Layout (old PCs):
        //   0: LoadLocal(0)
        //   1: JumpIfFalse(+2)        → target = 4 (Return)
        //   2: Const(0)               ← dropped by Rule 1
        //   3: Add                    ← dropped by Rule 1
        //   4: Return
        //
        // After optimize:
        //   0: LoadLocal(0)
        //   1: JumpIfFalse(+0)        → new target = 2 (Return)
        //   2: Return
        let mut chunk = mk_chunk(
            &[
                Op::LoadLocal(0),
                Op::JumpIfFalse(2),
                Op::Const(0),
                Op::Add,
                Op::Return,
            ],
            vec![Value::Int(0)],
            &[1, 1, 1, 1, 2],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), 3);
        assert!(matches!(chunk.code[0], Op::LoadLocal(0)));
        assert!(matches!(chunk.code[2], Op::Return));
        match chunk.code[1] {
            Op::JumpIfFalse(o) => {
                assert_eq!(o, 0, "jump must still land on the Return at new PC 2")
            }
            other => panic!("expected JumpIfFalse, got {:?}", other),
        }
    }

    #[test]
    fn optimize_preserves_line_info_length() {
        // Invariant: `line_info` must always have the same length
        // as `code` after any peephole transformation.
        let mut chunk = mk_chunk(
            &[
                Op::LoadLocal(0),
                Op::Const(0),
                Op::Add,
                Op::StoreLocal(0),
                Op::Jump(0),
                Op::Not,
                Op::JumpIfFalse(1),
                Op::Return,
            ],
            vec![Value::Int(1)],
            &[1, 2, 3, 4, 5, 6, 7, 8],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), chunk.line_info.len());
    }

    // ---------- Result return ----------

    #[test]
    fn optimize_returns_ok_for_normal_chunk() {
        // Verify that `optimize` returns `Ok(())` for a basic valid chunk.
        let mut chunk = mk_chunk(&[Op::Const(0), Op::Return], vec![Value::Int(42)], &[1, 1]);
        assert!(optimize(&mut chunk).is_ok());
    }

    // ---------- Rule 5: Const(1); Mul identity ----------

    #[test]
    fn rule5_fires_on_const_one_mul() {
        let chunk = mk_chunk(&[Op::Const(0), Op::Mul], vec![Value::Int(1)], &[1, 1]);
        assert!(rule_mul_one_identity(&chunk, 0, &[false; 3]));
    }

    #[test]
    fn rule5_skips_when_const_is_not_one() {
        let chunk = mk_chunk(&[Op::Const(0), Op::Mul], vec![Value::Int(2)], &[1, 1]);
        assert!(!rule_mul_one_identity(&chunk, 0, &[false; 3]));
    }

    #[test]
    fn rule5_skips_when_mul_is_jump_target() {
        let chunk = mk_chunk(&[Op::Const(0), Op::Mul], vec![Value::Int(1)], &[1, 1]);
        let mut targets = vec![false; 3];
        targets[1] = true;
        assert!(!rule_mul_one_identity(&chunk, 0, &targets));
    }

    #[test]
    fn rule5_drops_mul_one_in_full_pass() {
        // LoadLocal(0) * 1 should reduce to just LoadLocal(0).
        let mut chunk = mk_chunk(
            &[Op::LoadLocal(0), Op::Const(0), Op::Mul, Op::Return],
            vec![Value::Int(1)],
            &[1, 1, 1, 2],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code, vec![Op::LoadLocal(0), Op::Return]);
        assert_eq!(chunk.line_info, vec![1, 2]);
    }

    // ---------- Rule 6: Const(0); Mul → Const(0) (pure preceding load) ----------

    #[test]
    fn rule6_fires_when_preceding_emit_is_load_local() {
        let chunk = mk_chunk(
            &[Op::LoadLocal(0), Op::Const(0), Op::Mul],
            vec![Value::Int(0)],
            &[1, 1, 1],
        );
        // Simulate that we have already emitted LoadLocal(0).
        let emitted = vec![Op::LoadLocal(0)];
        // The pattern window starts at i=1 (Const(0); Mul).
        assert!(rule_mul_zero(&chunk, 1, &[false; 4], &emitted));
    }

    #[test]
    fn rule6_fires_when_preceding_emit_is_const() {
        // constants[0]=Int(42), constants[1]=Int(0)
        // code: [Const(0), Const(1), Mul]  — so at i=1 we have Const(1) → Int(0)
        let chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Mul],
            vec![Value::Int(42), Value::Int(0)],
            &[1, 1, 1],
        );
        let emitted = vec![Op::Const(0)];
        assert!(rule_mul_zero(&chunk, 1, &[false; 4], &emitted));
    }

    #[test]
    fn rule6_skips_when_preceding_emit_is_not_pure() {
        // If the previous emitted op was Add (a side-effect op that pops two
        // values and pushes one), we don't know whether the stack result is
        // the only value we'd be discarding, so the rule must not fire.
        let chunk = mk_chunk(
            &[Op::Add, Op::Const(0), Op::Mul],
            vec![Value::Int(0)],
            &[1, 1, 1],
        );
        let emitted = vec![Op::Add];
        assert!(!rule_mul_zero(&chunk, 1, &[false; 4], &emitted));
    }

    #[test]
    fn rule6_skips_when_const_is_not_zero() {
        let chunk = mk_chunk(
            &[Op::LoadLocal(0), Op::Const(0), Op::Mul],
            vec![Value::Int(5)],
            &[1, 1, 1],
        );
        let emitted = vec![Op::LoadLocal(0)];
        assert!(!rule_mul_zero(&chunk, 1, &[false; 4], &emitted));
    }

    #[test]
    fn rule6_skips_when_mul_is_jump_target() {
        let chunk = mk_chunk(
            &[Op::LoadLocal(0), Op::Const(0), Op::Mul],
            vec![Value::Int(0)],
            &[1, 1, 1],
        );
        let emitted = vec![Op::LoadLocal(0)];
        let mut targets = vec![false; 4];
        targets[2] = true; // Mul at pc=2 is a jump target
        assert!(!rule_mul_zero(&chunk, 1, &targets, &emitted));
    }

    #[test]
    fn rule6_folds_load_mul_zero_in_full_pass() {
        // LoadLocal(0) * 0 → Const(0)
        // chunk: [LoadLocal(0), Const(0), Mul, Return]
        // constants: [Int(0)]
        let mut chunk = mk_chunk(
            &[Op::LoadLocal(0), Op::Const(0), Op::Mul, Op::Return],
            vec![Value::Int(0)],
            &[1, 1, 1, 2],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code, vec![Op::Const(0), Op::Return]);
        assert_eq!(chunk.line_info.len(), chunk.code.len());
    }

    #[test]
    fn rule6_folds_const_mul_zero_in_full_pass() {
        // Const(42) * 0 → Const(0)
        // constants[0]=Int(42), constants[1]=Int(0)
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Mul, Op::Return],
            vec![Value::Int(42), Value::Int(0)],
            &[1, 1, 1, 2],
        );
        optimize(&mut chunk).unwrap();
        // Result: Const(1) (the zero), Return
        assert_eq!(chunk.code.len(), 2);
        assert!(matches!(chunk.code[0], Op::Const(_)));
        assert!(matches!(chunk.code[1], Op::Return));
        // The Const must refer to the zero.
        if let Op::Const(k) = chunk.code[0] {
            assert!(
                matches!(chunk.constants[k as usize], Value::Int(0)),
                "expected constant to be Int(0)"
            );
        }
        assert_eq!(chunk.line_info.len(), chunk.code.len());
    }
}
