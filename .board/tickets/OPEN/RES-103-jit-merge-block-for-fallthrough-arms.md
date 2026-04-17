---
id: RES-103
title: JIT merge block lifts both-arms-must-return restriction (RES-072 Phase F)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
RES-102 (Phase E) shipped if/else with `brif` but enforced a
strict shape: both arms must end in a return. This ticket lifts
that by introducing a `merge_block` — when a branch falls
through, it `jump`s to the merge block, the merge block becomes
the new active block, and statements after the if continue
lowering there. No phi nodes are needed because Phase E/F still
returns `()` from `lower_if_statement` (the if itself doesn't
produce an SSA value yet — that's a future "if as expression"
phase).

After this ticket the JIT can compile programs like:

```
if (x > 0) {
    return 1;
}
return -1;
```

…where the then-arm returns and the trailing `return -1;` is
the fallthrough.

## Acceptance criteria
- `lower_if_statement` grows a `merge_block`:
  - When then-arm doesn't terminate, emit `bcx.ins().jump(merge_block, &[])`.
  - When else-arm doesn't terminate (or is missing entirely),
    emit the same jump from the else_block.
  - After both arms processed, switch to merge_block and
    seal it. The function builder is now positioned to lower
    the trailing statements.
  - If BOTH arms terminate (the Phase E case), no merge_block
    is needed — short-circuit and don't create one.
- `compile_node_list` no longer returns `Ok(true)` immediately
  on IfStatement: an if where both arms terminate still
  returns `Ok(true)` (no statements after it can run), but an
  if with a fallthrough returns `Ok(false)` so the loop keeps
  walking the trailing statements.
- New unit tests in `jit_backend::tests`:
  - `jit_if_then_returns_else_falls_through`:
    `if (1 < 2) { return 7; } return 9;` → 7 (then-arm taken)
  - `jit_if_else_returns_then_falls_through`:
    `if (1 > 2) { return 7; } return 9;` → 9 (fallthrough,
    actually the else is also missing — bare-if + fallthrough)
  - `jit_bare_if_with_fallthrough`:
    `if (false) { return 7; } return 9;` → 9 (no else,
    condition false → straight to fallthrough)
  - `jit_if_with_no_terminator_unsupported`:
    `if (cond) { /* let x */ } /* nothing else */` → still
    Unsupported (function never returns), descriptor names
    "function never returns".
- The previously-introduced `jit_rejects_if_without_else` test
  needs adapting — bare `if` + fallthrough now works. Repurpose
  to a different boundary case (e.g., if where neither arm
  has a return AND no trailing statement → function never
  returns → Unsupported).
- The `jit_rejects_if_arm_without_return` test should be
  retired or repurposed — Phase F removes the restriction
  it pinned. Pick a new restriction to test.
- Smoke test in `tests/examples_smoke.rs` (gated `--features jit`):
  `if (5 < 3) { return 7; } return 9;` → driver prints 9, exits 0.
- All four feature configs pass cargo test + clippy --
  -D warnings.
- Commit message: `RES-103: JIT merge block + fallthrough (RES-072 Phase F)`.

## Notes
- Cranelift API:
  ```rust
  let merge_block = bcx.create_block();
  // ... after lowering each arm ...
  if !then_terminated {
      bcx.ins().jump(merge_block, &[]);
  }
  // ... else-arm processing ...
  if !else_terminated {
      bcx.ins().jump(merge_block, &[]);
  }
  bcx.switch_to_block(merge_block);
  bcx.seal_block(merge_block);
  ```
- The seal is fine immediately after the jumps because both
  predecessors are now known.
- Don't try to make `if` produce a value (`x = if cond { 1 }
  else { 2 }`) in this ticket — that's a separate ticket
  needing block params + phi-style merging. Phase F is purely
  about statement-level fallthrough.

## Log
- 2026-04-17 created by manager (Phase F scope, follow-up to RES-102)
