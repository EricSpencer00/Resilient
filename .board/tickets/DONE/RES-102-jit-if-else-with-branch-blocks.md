---
id: RES-102
title: JIT lowers if/else with cranelift blocks (RES-072 Phase E)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Builds on RES-099 (arith) and RES-100 (comparisons + bool literals)
to give the JIT real control flow. After this ticket the JIT can
compile programs like:

```
if (x > 0) {
    return 1;
} else {
    return -1;
}
```

The driver shape grows from "single top-level return" to
"top-level if/else where each arm contains a return, optionally
followed by a trailing top-level return as a fallthrough." This
is the smallest shape that meaningfully exercises Cranelift
blocks + brif + jump.

## Acceptance criteria
- New helper `lower_block` (private, in jit_backend.rs) that
  walks a slice of statements and lowers the first
  `ReturnStatement` it finds in that block, emitting the
  Cranelift `return_` for the JIT'd function. Returns
  `Err(JitError::Unsupported(...))` if it doesn't find a
  return — Phase E doesn't yet handle "if/else where a branch
  has no return".
- New helper `lower_if_statement` (private) that:
  - Takes the cond expression, then-block, and optional else-block
  - Calls `lower_expr` on the condition (which must be an i64 0/1
    after the icmp lowering — RES-100 made this true for comparison
    ops and bool literals)
  - Creates two cranelift blocks: `then_block`, `else_block`
  - Creates a `merge_block` only if BOTH branches fall through
    (i.e. neither contains a return). For Phase E, since we
    require both arms to end in a return, no merge_block is
    needed — the function exits from each arm.
  - Emits `brif(cond, then_block, &[], else_block, &[])`
  - Switches to each block, lowers its body, and seals each.
- Extend `top_level_return_expr` → rename to `compile_program`
  (or similar) that walks the program's statements and:
  - If it sees an `IfStatement`, lower it via `lower_if_statement`
    and stop (the if exits both arms — no further code is reachable
    in Phase E's shape).
  - If it sees a `ReturnStatement`, lower the trailing return.
  - Otherwise return `Unsupported`.
- New unit tests in `jit_backend::tests`:
  - `jit_if_then_returns`: `if (1 < 2) { return 7; } return 9;` → 7
  - `jit_if_else_returns`: `if (1 > 2) { return 7; } else { return 9; }` → 9
  - `jit_if_with_arith_cond`: `if (5 + 5 == 10) { return 1; } return 0;` → 1
  - `jit_nested_if_unsupported_for_now`: confirms a nested
    `if` inside the then-arm returns Unsupported — Phase F can
    lift this restriction.
- Smoke test in `tests/examples_smoke.rs` (gated `--features jit`):
  `if (3 < 7) { return 42; } return 0;` → driver prints 42, exit 0.
- `cargo test`, `cargo clippy --all-targets -- -D warnings` pass
  on default / z3 / lsp / jit configs.
- Commit message: `RES-102: JIT lowers if/else with brif (RES-072 Phase E)`.

## Notes
- Cranelift block dance reference:
  ```rust
  let then_block = bcx.create_block();
  let else_block = bcx.create_block();
  bcx.ins().brif(cond, then_block, &[], else_block, &[]);

  bcx.switch_to_block(then_block);
  bcx.seal_block(then_block);
  // lower then-arm; emit return_

  bcx.switch_to_block(else_block);
  bcx.seal_block(else_block);
  // lower else-arm; emit return_
  ```
- The `seal_block` calls are critical — Cranelift refuses to
  finalize otherwise. With no block params and no back-edges,
  they can be sealed immediately after creation/branch.
- Phase F (likely RES-103) will lift the "both arms must return"
  restriction by introducing `merge_block` + block params. Don't
  try to ship that here — the merge case needs phi nodes and is a
  meaningful step up in cranelift complexity.
- Watch for `IfStatement`'s field shape in the AST — it's
  `Node::IfStatement { condition, then_branch, else_branch, span }`
  per the existing main.rs definition. else_branch is `Option<Box<Node>>`
  pointing at a Block (or nested If for `else if`).

## Log
- 2026-04-17 created by manager (Phase E scope)
- 2026-04-17 executor: jit_backend refactored from "single
  return expression" to "walk top-level statements." New helpers:
  compile_statements (Spanned wrapper), compile_node_list (raw Node
  walker that returns Ok(true) when a terminator was emitted),
  lower_if_statement (brif into then/else cranelift blocks),
  lower_block_or_stmt (Block/IfStatement/ReturnStatement
  delegate). Returned-bool is the terminator-detection mechanism
  since cranelift's is_filled is private — the helpers thread it
  back to the caller, who decides whether to error (top-level)
  or fall through (inside a Block, future phase).
  Phase E enforces: both arms of every if must end in a return.
  Bare `if` (no else) and arms without returns return
  Unsupported with descriptive messages so users see the gap.
  Seven new unit tests cover: then-arm, else-arm, arith
  condition, bool-literal condition, nested if in then-arm,
  bare-if rejection, arm-without-return rejection. Smoke test
  bytecode_jit_runs_if_else added: `if (3 < 7) { return 42; }
  else { return 0; }` → driver prints 42, exits 0.
  Field name correction vs ticket draft: AST uses
  `condition`/`consequence`/`alternative` (not the `then_branch`/
  `else_branch` the ticket guessed). The else_branch shape
  (`Option<Box<Node>>` pointing at a Block, or a nested If for
  `else if`) matches the ticket. Matrix: default 217, z3 225,
  lsp 221, jit 245 — clippy clean across all four.
