---
id: RES-168
title: JIT: tail-call optimization for direct self-recursion
state: IN_PROGRESS
priority: P3
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Tail calls matter for functional-style programs — without TCO,
deep recursion overflows the host stack. Start small: only direct
self-recursion in tail position, same signature. Mutual TCO and
cross-function tail calls are harder and can wait.

## Acceptance criteria
- Lowering detects `return fn_name(args)` where `fn_name` is the
  enclosing function and arg count matches.
- Lowers to: evaluate args into the parameter variables, then
  `bcx.ins().jump(entry_block, &[])`. No `return` instruction is
  emitted on this path.
- SSA handling: the parameter Variables are re-`def_var`'d; phi
  construction at `entry_block` is handled by Cranelift's
  FunctionBuilder since entry now has ≥ 2 preds (original + tail
  back-edge).
- Unit tests: tail-recursive `count(n)` reaches `n = 1_000_000`
  without stack overflow; non-tail-position call to self is NOT
  optimized (still uses regular call, still stacks up).
- `benchmarks/jit/tail_rec.rs` added; numbers in RESULTS.md.
- Commit message: `RES-168: JIT TCO for direct self-recursion`.

## Notes
- Don't conflate with `tailcall` instruction (Cranelift 0.101+
  proposal) — we're doing the equivalent with a jump to the entry
  block, which is portable.
- Skip over tail calls wrapped in a `live` block — the retry
  semantics (RES-036) mean the call is NOT in tail position; the
  block boundary requires the activation.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
