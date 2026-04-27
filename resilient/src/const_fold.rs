//! RES-298: bytecode constant-folding pass.
//!
//! Runs after the compiler emits a chunk and before the peephole
//! optimizer. Collapses arithmetic, comparison, and logical ops over
//! literal operands into a single `Op::Const`. The peephole pass that
//! follows then has access to a chunk where pure-constant expressions
//! have already been reduced — opening up identity-fold opportunities
//! (`Const 0; Add`, `Const 1; Mul`, etc.) that wouldn't exist on the
//! raw emitter output.
//!
//! ## Folded patterns
//!
//! Binary integer arithmetic and comparisons (3 ops → 1):
//!
//! | Pattern | Result |
//! |---|---|
//! | `Const(i); Const(j); Add` | `Const(i + j)` (wrapping) |
//! | `Const(i); Const(j); Sub` | `Const(i - j)` (wrapping) |
//! | `Const(i); Const(j); Mul` | `Const(i * j)` (wrapping) |
//! | `Const(i); Const(j); Div` | `Const(i / j)` — skipped if `j == 0` |
//! | `Const(i); Const(j); Mod` | `Const(i % j)` — skipped if `j == 0` |
//! | `Const(i); Const(j); Eq`/`Neq`/`Lt`/`Le`/`Gt`/`Ge` | `Const(bool)` |
//!
//! Unary ops (2 ops → 1):
//!
//! | Pattern | Result |
//! |---|---|
//! | `Const(i); Neg` | `Const(-i)` (wrapping) |
//! | `Const(b); Not` | `Const(!b)` |
//!
//! Pure-builtin folds (2 ops → 1):
//!
//! | Pattern | Result |
//! |---|---|
//! | `Const(s); CallBuiltin { name="len", arity=1 }` | `Const(s.chars().count())` |
//!
//! ## Semantic fidelity
//!
//! - Integer arithmetic is folded with `wrapping_*`, matching the
//!   VM's dispatch (`Op::Add` etc. use `wrapping_add` / `wrapping_sub`
//!   / `wrapping_mul`). Overflow does not trap at the VM, so folding
//!   does not change observable behavior.
//! - `Div` and `Mod` are NOT folded when the divisor is `0`: the VM
//!   raises `VmError::DivideByZero`. Leaving the op in place preserves
//!   the trap and the originating source line.
//! - `Float`, `Bool && Bool`, `Bool || Bool`, and string-concat folds
//!   are NOT performed: the bytecode VM's arithmetic ops accept only
//!   integers, `&&` / `||` are lowered to control flow (not a single
//!   binop), and there is no `Concat` opcode. Adding folds for those
//!   would not match any pattern actually emitted by the compiler.
//! - `len` is folded only over literal `Value::String` arguments, not
//!   over `Value::Array` literals — array literals lower to `MakeArray`
//!   (multi-op), so the simple `Const; CallBuiltin` window does not
//!   match. No correctness concern: the unfolded form runs fine in
//!   the VM.
//! - Non-pure builtins (`println`, anything I/O-touching) are never
//!   folded — only `len` is on the allow-list.
//!
//! ## Jump-target safety
//!
//! Folding shortens the instruction stream, so jumps that target
//! interior PCs of a fold pattern would be stranded. We compute the
//! set of jump-target PCs once per pass; if any interior position of
//! a candidate window is in the set, the fold is skipped for that
//! site. Jumps that target the FIRST op of a window (the Const that
//! becomes the replacement) remain valid — the new Const lands at
//! the same effective position via the old → new PC fixup table.
//!
//! Mechanically identical to the relinking discipline in `peephole.rs`:
//! the Result-returning `optimize` function is the only public entry
//! point; the per-rule predicates are visible to `mod tests` so each
//! one can be exercised in isolation.
//!
//! ## Iteration
//!
//! A single linear scan over the chunk only folds windows that are
//! already adjacent. `2 + 3 * 4` lowers to
//! `Const(2); Const(3); Const(4); Mul; Add` — the first scan folds
//! the inner `Mul` into `Const(12)`, leaving `Const(2); Const(12); Add`,
//! which is itself a fold candidate. We iterate to fixpoint (capped at
//! a small constant) so the user's expectation that "all fully-constant
//! arithmetic collapses" holds without a separate worklist algorithm.

use crate::Value;
use crate::bytecode::{Chunk, Op};

/// Errors from the constant-folding pass.
#[derive(Debug)]
pub enum FoldError {
    InternalError(&'static str),
}

impl std::fmt::Display for FoldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FoldError::InternalError(msg) => write!(f, "constant folding: {}", msg),
        }
    }
}

impl std::error::Error for FoldError {}

/// Hard cap on fold iterations. Each pass is O(n); the body of any
/// realistic chunk reaches fixpoint in a handful of passes (one per
/// nesting level of constant arithmetic). This cap is purely a
/// safety net — if it ever fires, that's an internal bug and we
/// surface it instead of looping forever.
const MAX_PASSES: usize = 64;

/// Top-level entry. Iterates [`fold_pass`] until no further folds fire,
/// or until the safety cap is hit. Idempotent on chunks that already
/// have no foldable windows.
pub fn optimize(chunk: &mut Chunk) -> Result<(), FoldError> {
    for _ in 0..MAX_PASSES {
        let folded_any = fold_pass(chunk)?;
        if !folded_any {
            return Ok(());
        }
    }
    Err(FoldError::InternalError("fold did not reach fixpoint"))
}

/// Pipeline-aware entry point. Runs [`optimize`] only when the
/// `RESILIENT_CONST_FOLD` environment variable is set to `1`; otherwise
/// returns `Ok(())` without touching the chunk.
///
/// This indirection exists because turning constant folding on by
/// default would change the bytecode shape that one pre-existing
/// compiler test pins (`compile_arith_respects_precedence` asserts
/// the un-folded `Const Const Const Mul Add` sequence for
/// `2 + 3 * 4`). That test predates this ticket and the project's
/// test-protection policy requires maintainer approval to update it.
/// Until the test is adjusted in a follow-up, the fold pass is
/// shipped as a fully-tested but opt-in optimization. To enable in
/// any compile pipeline (CLI, REPL, JIT codegen frontend), run
/// with `RESILIENT_CONST_FOLD=1`.
pub fn optimize_if_enabled(chunk: &mut Chunk) -> Result<(), FoldError> {
    if std::env::var("RESILIENT_CONST_FOLD").as_deref() == Ok("1") {
        optimize(chunk)?;
    }
    Ok(())
}

/// One linear scan. Returns `true` if at least one fold fired (the
/// caller re-runs the pass to catch newly-adjacent windows). On
/// `false` the chunk is at a local fixpoint for the rules below.
fn fold_pass(chunk: &mut Chunk) -> Result<bool, FoldError> {
    let targets = jump_targets(chunk);

    // Capture each jump's ORIGINAL target PC up-front so we can
    // remap through old_to_new at the end. Identical strategy to
    // peephole.rs.
    let orig_targets: Vec<Option<usize>> = chunk
        .code
        .iter()
        .enumerate()
        .map(|(pc, op)| jump_target_pc(*op, pc))
        .collect();

    let mut new_code: Vec<Op> = Vec::with_capacity(chunk.code.len());
    let mut new_line_info: Vec<u32> = Vec::with_capacity(chunk.code.len());
    let mut old_to_new: Vec<usize> = vec![usize::MAX; chunk.code.len() + 1];
    let mut folded_any = false;

    let mut i = 0;
    while i < chunk.code.len() {
        old_to_new[i] = new_code.len();

        // Three-op windows first (binary ops). These shadow the
        // two-op windows: a `Const Const Add` shouldn't be split
        // into a `Const + Const Add` two-op match.
        if let Some((value, line)) = try_fold_binop(chunk, i, &targets) {
            let k = chunk.add_constant(value).map_err(|_| {
                FoldError::InternalError("constant pool overflow during binop fold")
            })?;
            // Map the second and third ops of the window so that
            // any jump landing on the binary op or its second
            // operand is rewritten to the replacement Const. In
            // practice the `targets` check above prevents that
            // case; the mapping is defensive.
            old_to_new[i + 1] = new_code.len();
            old_to_new[i + 2] = new_code.len();
            new_code.push(Op::Const(k));
            new_line_info.push(line);
            i += 3;
            folded_any = true;
            continue;
        }

        // Two-op windows (unary ops, len-of-literal).
        if let Some((value, line)) = try_fold_unop(chunk, i, &targets) {
            let k = chunk
                .add_constant(value)
                .map_err(|_| FoldError::InternalError("constant pool overflow during unop fold"))?;
            old_to_new[i + 1] = new_code.len();
            new_code.push(Op::Const(k));
            new_line_info.push(line);
            i += 2;
            folded_any = true;
            continue;
        }

        if let Some((value, line)) = try_fold_len(chunk, i, &targets) {
            let k = chunk
                .add_constant(value)
                .map_err(|_| FoldError::InternalError("constant pool overflow during len fold"))?;
            old_to_new[i + 1] = new_code.len();
            new_code.push(Op::Const(k));
            new_line_info.push(line);
            i += 2;
            folded_any = true;
            continue;
        }

        // No rule fired — copy verbatim.
        new_code.push(chunk.code[i]);
        new_line_info.push(chunk.line_info[i]);
        i += 1;
    }
    old_to_new[chunk.code.len()] = new_code.len();

    if !folded_any {
        return Ok(false);
    }

    // Re-link jump offsets in the rewritten stream. Identical
    // mechanic to peephole.rs::optimize: find the originating old
    // PC for each new jump, look up its original target, map
    // through old_to_new, and recompute the relative offset.
    for (new_pc, op) in new_code.iter_mut().enumerate() {
        if !is_jump_op(*op) {
            continue;
        }
        let Some(old_pc) = (0..chunk.code.len()).find(|&p| old_to_new[p] == new_pc) else {
            return Err(FoldError::InternalError(
                "constant fold: new_pc with no originating old_pc",
            ));
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
    Ok(true)
}

// ---------- jump bookkeeping (mirrors peephole.rs) ----------

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

// ---------- per-rule predicates ----------

/// Try to fold a `Const Const <binop>` window. Returns `Some((Value,
/// line))` if the window is foldable; the caller emits one
/// `Op::Const` and consumes 3 source ops.
///
/// `line` is taken from the FIRST op of the window so VM runtime
/// errors after subsequent passes still blame a sane source line.
pub(crate) fn try_fold_binop(chunk: &Chunk, i: usize, targets: &[bool]) -> Option<(Value, u32)> {
    if i + 2 >= chunk.code.len() {
        return None;
    }
    let Op::Const(k0) = chunk.code[i] else {
        return None;
    };
    let Op::Const(k1) = chunk.code[i + 1] else {
        return None;
    };
    let op = chunk.code[i + 2];

    // Skip the fold if any interior position of the window is a
    // jump target. Position `i` is fine — jumps to it land on the
    // replacement Const which has the same effective behavior
    // (push one value).
    if *targets.get(i + 1).unwrap_or(&false) || *targets.get(i + 2).unwrap_or(&false) {
        return None;
    }

    let lhs = chunk.constants.get(k0 as usize)?;
    let rhs = chunk.constants.get(k1 as usize)?;

    let folded = match (lhs, rhs, op) {
        (Value::Int(a), Value::Int(b), Op::Add) => Value::Int(a.wrapping_add(*b)),
        (Value::Int(a), Value::Int(b), Op::Sub) => Value::Int(a.wrapping_sub(*b)),
        (Value::Int(a), Value::Int(b), Op::Mul) => Value::Int(a.wrapping_mul(*b)),
        // Skip div/mod by zero — preserve runtime trap & its source line.
        (Value::Int(_), Value::Int(0), Op::Div) | (Value::Int(_), Value::Int(0), Op::Mod) => {
            return None;
        }
        // i64::MIN / -1 also traps in non-wrapping division. The
        // VM uses `/` directly (not wrapping_div), so leave that
        // window for the runtime to handle and don't fold it.
        (Value::Int(i64::MIN), Value::Int(-1), Op::Div)
        | (Value::Int(i64::MIN), Value::Int(-1), Op::Mod) => {
            return None;
        }
        (Value::Int(a), Value::Int(b), Op::Div) => Value::Int(a / b),
        (Value::Int(a), Value::Int(b), Op::Mod) => Value::Int(a % b),
        (Value::Int(a), Value::Int(b), Op::Eq) => Value::Bool(a == b),
        (Value::Int(a), Value::Int(b), Op::Neq) => Value::Bool(a != b),
        (Value::Int(a), Value::Int(b), Op::Lt) => Value::Bool(a < b),
        (Value::Int(a), Value::Int(b), Op::Le) => Value::Bool(a <= b),
        (Value::Int(a), Value::Int(b), Op::Gt) => Value::Bool(a > b),
        (Value::Int(a), Value::Int(b), Op::Ge) => Value::Bool(a >= b),
        // Bool comparisons via Eq/Neq are emitted by the compiler
        // when both sides are bool literals (`true == false`).
        (Value::Bool(a), Value::Bool(b), Op::Eq) => Value::Bool(a == b),
        (Value::Bool(a), Value::Bool(b), Op::Neq) => Value::Bool(a != b),
        _ => return None,
    };
    Some((folded, chunk.line_info[i]))
}

/// Try to fold a `Const <unop>` window. Returns `Some((Value, line))`
/// on a match; the caller emits one `Op::Const` and consumes 2
/// source ops.
pub(crate) fn try_fold_unop(chunk: &Chunk, i: usize, targets: &[bool]) -> Option<(Value, u32)> {
    if i + 1 >= chunk.code.len() {
        return None;
    }
    let Op::Const(k) = chunk.code[i] else {
        return None;
    };
    let op = chunk.code[i + 1];
    if *targets.get(i + 1).unwrap_or(&false) {
        return None;
    }

    let v = chunk.constants.get(k as usize)?;
    let folded = match (v, op) {
        // `i64::MIN.wrapping_neg() == i64::MIN`, matches VM behavior.
        (Value::Int(a), Op::Neg) => Value::Int(a.wrapping_neg()),
        (Value::Bool(b), Op::Not) => Value::Bool(!b),
        _ => return None,
    };
    Some((folded, chunk.line_info[i]))
}

/// Try to fold `Const(string); CallBuiltin { name="len", arity=1 }`
/// into `Const(int)`. The arity check is strict — `len(x, y)` is a
/// type error elsewhere; here we just refuse to fold. Returns
/// `Some((Value, line))` on a match.
pub(crate) fn try_fold_len(chunk: &Chunk, i: usize, targets: &[bool]) -> Option<(Value, u32)> {
    if i + 1 >= chunk.code.len() {
        return None;
    }
    let Op::Const(k) = chunk.code[i] else {
        return None;
    };
    let Op::CallBuiltin { name_const, arity } = chunk.code[i + 1] else {
        return None;
    };
    if arity != 1 {
        return None;
    }
    if *targets.get(i + 1).unwrap_or(&false) {
        return None;
    }
    let name = chunk.constants.get(name_const as usize)?;
    let Value::String(name_str) = name else {
        return None;
    };
    if name_str != "len" {
        return None;
    }
    let arg = chunk.constants.get(k as usize)?;
    let Value::String(s) = arg else {
        return None;
    };
    Some((Value::Int(s.chars().count() as i64), chunk.line_info[i]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_chunk(code: &[Op], constants: Vec<Value>, lines: &[u32]) -> Chunk {
        Chunk {
            code: code.to_vec(),
            constants,
            line_info: lines.to_vec(),
        }
    }

    /// `Value` does not implement `PartialEq` (it carries closures
    /// and other non-comparable variants), so the tests here unwrap
    /// the inner literal manually before asserting on the primitive.
    fn unwrap_int(v: &Value) -> i64 {
        match v {
            Value::Int(i) => *i,
            other => panic!("expected Value::Int, got {:?}", other),
        }
    }

    fn unwrap_bool(v: &Value) -> bool {
        match v {
            Value::Bool(b) => *b,
            other => panic!("expected Value::Bool, got {:?}", other),
        }
    }

    // ---------- binop folds ----------

    #[test]
    fn folds_int_add() {
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
            vec![Value::Int(2), Value::Int(3)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        // After fold: a single Const referring to 5, then Return.
        assert_eq!(chunk.code.len(), 2);
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const, got {:?}", chunk.code[0]);
        };
        assert_eq!(unwrap_int(&chunk.constants[k as usize]), 5);
        assert!(matches!(chunk.code[1], Op::Return));
    }

    #[test]
    fn folds_int_sub() {
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Sub, Op::Return],
            vec![Value::Int(10), Value::Int(7)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const");
        };
        assert_eq!(unwrap_int(&chunk.constants[k as usize]), 3);
    }

    #[test]
    fn folds_int_mul() {
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Mul, Op::Return],
            vec![Value::Int(6), Value::Int(7)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const");
        };
        assert_eq!(unwrap_int(&chunk.constants[k as usize]), 42);
    }

    #[test]
    fn folds_int_div() {
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Div, Op::Return],
            vec![Value::Int(20), Value::Int(4)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const");
        };
        assert_eq!(unwrap_int(&chunk.constants[k as usize]), 5);
    }

    #[test]
    fn does_not_fold_int_div_by_zero() {
        // The runtime traps on div-by-zero — folding would silently
        // discard the trap. Leave the op so the VM raises
        // `VmError::DivideByZero` at the original source line.
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Div, Op::Return],
            vec![Value::Int(20), Value::Int(0)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), 4);
        assert!(matches!(chunk.code[2], Op::Div));
    }

    #[test]
    fn does_not_fold_int_mod_by_zero() {
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Mod, Op::Return],
            vec![Value::Int(7), Value::Int(0)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), 4);
        assert!(matches!(chunk.code[2], Op::Mod));
    }

    #[test]
    fn does_not_fold_int_min_div_neg_one() {
        // i64::MIN / -1 overflows in non-wrapping division (Rust
        // panics in debug, wraps in release). The VM uses `/` not
        // `wrapping_div`, so leave it for the runtime to handle.
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Div, Op::Return],
            vec![Value::Int(i64::MIN), Value::Int(-1)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), 4);
    }

    #[test]
    fn folds_int_add_with_overflow_wraps() {
        // Wrapping arithmetic at fold time matches the VM's
        // `wrapping_add` dispatch — no observable behavior change.
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
            vec![Value::Int(i64::MAX), Value::Int(1)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const");
        };
        assert_eq!(unwrap_int(&chunk.constants[k as usize]), i64::MIN);
    }

    #[test]
    fn folds_int_comparison_eq() {
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Eq, Op::Return],
            vec![Value::Int(7), Value::Int(7)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const");
        };
        assert!(unwrap_bool(&chunk.constants[k as usize]));
    }

    #[test]
    fn folds_int_comparison_lt() {
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Lt, Op::Return],
            vec![Value::Int(2), Value::Int(5)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const");
        };
        assert!(unwrap_bool(&chunk.constants[k as usize]));
    }

    #[test]
    fn folds_bool_eq() {
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Eq, Op::Return],
            vec![Value::Bool(true), Value::Bool(false)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const");
        };
        assert!(!unwrap_bool(&chunk.constants[k as usize]));
    }

    // ---------- unary folds ----------

    #[test]
    fn folds_int_neg() {
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Neg, Op::Return],
            vec![Value::Int(42)],
            &[1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const");
        };
        assert_eq!(unwrap_int(&chunk.constants[k as usize]), -42);
    }

    #[test]
    fn folds_bool_not() {
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Not, Op::Return],
            vec![Value::Bool(true)],
            &[1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const");
        };
        assert!(!unwrap_bool(&chunk.constants[k as usize]));
    }

    #[test]
    fn does_not_fold_neg_on_bool() {
        // Type-mismatch — left for the runtime to raise. Folder
        // refuses to invent a value out of thin air.
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Neg, Op::Return],
            vec![Value::Bool(true)],
            &[1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), 3);
    }

    // ---------- len builtin fold ----------

    #[test]
    fn folds_len_of_string_literal() {
        // constants: [0]="hello", [1]="len"
        let mut chunk = mk_chunk(
            &[
                Op::Const(0),
                Op::CallBuiltin {
                    name_const: 1,
                    arity: 1,
                },
                Op::Return,
            ],
            vec![
                Value::String("hello".to_string()),
                Value::String("len".to_string()),
            ],
            &[1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), 2);
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const");
        };
        assert_eq!(unwrap_int(&chunk.constants[k as usize]), 5);
    }

    #[test]
    fn folds_len_counts_chars_not_bytes() {
        // Multi-byte UTF-8: "naïve" has 5 chars, 6 bytes.
        let mut chunk = mk_chunk(
            &[
                Op::Const(0),
                Op::CallBuiltin {
                    name_const: 1,
                    arity: 1,
                },
                Op::Return,
            ],
            vec![
                Value::String("naïve".to_string()),
                Value::String("len".to_string()),
            ],
            &[1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const");
        };
        assert_eq!(unwrap_int(&chunk.constants[k as usize]), 5);
    }

    #[test]
    fn does_not_fold_other_builtins() {
        // `println("hi")` is impure I/O — never fold.
        let mut chunk = mk_chunk(
            &[
                Op::Const(0),
                Op::CallBuiltin {
                    name_const: 1,
                    arity: 1,
                },
                Op::Return,
            ],
            vec![
                Value::String("hi".to_string()),
                Value::String("println".to_string()),
            ],
            &[1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), 3);
        assert!(matches!(chunk.code[1], Op::CallBuiltin { .. }));
    }

    #[test]
    fn does_not_fold_len_of_non_string_constant() {
        // Folding only handles string literals — `len([1,2,3])`
        // doesn't even appear in this shape (arrays lower to
        // MakeArray), but the predicate must still refuse.
        let mut chunk = mk_chunk(
            &[
                Op::Const(0),
                Op::CallBuiltin {
                    name_const: 1,
                    arity: 1,
                },
                Op::Return,
            ],
            vec![Value::Int(42), Value::String("len".to_string())],
            &[1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), 3);
    }

    // ---------- iterative fixpoint ----------

    #[test]
    fn ticket_example_two_plus_three_times_four_emits_one_const() {
        // Per the ticket's acceptance criteria: `2 + 3 * 4`
        // compiles to `Const(2); Const(3); Const(4); Mul; Add` and
        // must reduce to a single Const(14) op.
        let mut chunk = mk_chunk(
            &[
                Op::Const(0),
                Op::Const(1),
                Op::Const(2),
                Op::Mul,
                Op::Add,
                Op::Return,
            ],
            vec![Value::Int(2), Value::Int(3), Value::Int(4)],
            &[1, 1, 1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        // Filter to ops that contribute to the result: a single
        // Const, then Return.
        assert_eq!(
            chunk.code.len(),
            2,
            "expected 1 Const + Return, got {:?}",
            chunk.code
        );
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const, got {:?}", chunk.code[0]);
        };
        assert_eq!(unwrap_int(&chunk.constants[k as usize]), 14);
        assert!(matches!(chunk.code[1], Op::Return));
    }

    #[test]
    fn nested_unary_then_binop_folds_to_fixpoint() {
        // `-(2 + 3)` — folds in two passes.
        let mut chunk = mk_chunk(
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Neg, Op::Return],
            vec![Value::Int(2), Value::Int(3)],
            &[1, 1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), 2);
        let Op::Const(k) = chunk.code[0] else {
            panic!("expected Const");
        };
        assert_eq!(unwrap_int(&chunk.constants[k as usize]), -5);
    }

    // ---------- safety: jump preservation ----------

    #[test]
    fn does_not_fold_when_binop_is_jump_target() {
        // A Jump targets PC=2 (the Add). Folding would strand the
        // jump, so the fold must skip this site.
        let mut chunk = mk_chunk(
            &[Op::Jump(1), Op::Const(0), Op::Const(1), Op::Add, Op::Return],
            vec![Value::Int(2), Value::Int(3)],
            &[1, 1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        // Wait: Jump(1) at PC=0 → target = 0+1+1 = 2. PC=2 is
        // Const(1), interior of the window i=1. Fold must skip.
        assert_eq!(chunk.code.len(), 5);
        assert!(matches!(chunk.code[3], Op::Add));
    }

    #[test]
    fn does_not_fold_when_unop_is_jump_target() {
        // Jump(1) at PC=0 lands on PC=2 (Neg, interior of i=1
        // unop window). Fold must skip.
        let mut chunk = mk_chunk(
            &[Op::Jump(1), Op::Const(0), Op::Neg, Op::Return],
            vec![Value::Int(7)],
            &[1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), 4);
        assert!(matches!(chunk.code[2], Op::Neg));
    }

    #[test]
    fn jumps_relink_across_folded_window() {
        // Forward JumpIfFalse skips over a foldable `Const Const Add`.
        // After folding, the jump's target must still land on the
        // post-window instruction.
        //
        // Old layout:
        //   0: LoadLocal(0)
        //   1: JumpIfFalse(+3)         → target = 5 (Return)
        //   2: Const(0)                ← folded
        //   3: Const(1)                ← folded
        //   4: Add                     ← folded → Const(merged)
        //   5: Return
        //
        // After fold:
        //   0: LoadLocal(0)
        //   1: JumpIfFalse(+1)         → target = 3 (Return)
        //   2: Const(merged)
        //   3: Return
        let mut chunk = mk_chunk(
            &[
                Op::LoadLocal(0),
                Op::JumpIfFalse(3),
                Op::Const(0),
                Op::Const(1),
                Op::Add,
                Op::Return,
            ],
            vec![Value::Int(2), Value::Int(3)],
            &[1, 1, 1, 1, 1, 1],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), 4);
        assert!(matches!(chunk.code[0], Op::LoadLocal(0)));
        let Op::Const(k) = chunk.code[2] else {
            panic!("expected Const at PC 2");
        };
        assert_eq!(unwrap_int(&chunk.constants[k as usize]), 5);
        assert!(matches!(chunk.code[3], Op::Return));
        match chunk.code[1] {
            Op::JumpIfFalse(o) => assert_eq!(o, 1, "jump must land on Return at new PC 3"),
            other => panic!("expected JumpIfFalse, got {:?}", other),
        }
    }

    // ---------- invariants ----------

    #[test]
    fn line_info_length_matches_code_length() {
        let mut chunk = mk_chunk(
            &[
                Op::Const(0),
                Op::Const(1),
                Op::Add,
                Op::Const(2),
                Op::Neg,
                Op::Return,
            ],
            vec![Value::Int(1), Value::Int(2), Value::Int(3)],
            &[10, 11, 12, 13, 14, 15],
        );
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code.len(), chunk.line_info.len());
    }

    #[test]
    fn empty_chunk_is_ok() {
        let mut chunk = mk_chunk(&[], vec![], &[]);
        assert!(optimize(&mut chunk).is_ok());
    }

    #[test]
    fn idempotent_on_already_folded_chunk() {
        let mut chunk = mk_chunk(&[Op::Const(0), Op::Return], vec![Value::Int(42)], &[1, 1]);
        optimize(&mut chunk).unwrap();
        // Second run: no changes.
        let before = chunk.code.clone();
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code, before);
    }

    #[test]
    fn unfoldable_chunk_passes_through_unchanged() {
        // Ops that aren't part of any fold pattern stay put.
        let mut chunk = mk_chunk(
            &[Op::LoadLocal(0), Op::LoadLocal(1), Op::Add, Op::Return],
            vec![],
            &[1, 1, 1, 1],
        );
        let before = chunk.code.clone();
        optimize(&mut chunk).unwrap();
        assert_eq!(chunk.code, before);
    }
}
