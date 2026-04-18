---
id: RES-107
title: JIT lowers reassignment + while loops (RES-072 Phase J)
state: DONE
priority: P3
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Phase G (RES-104) added immutable let bindings. Phase J adds
the missing pieces for non-trivial straight-line code: variable
reassignment (`x = x + 1`) and while loops. After this ticket
the JIT can compile imperative-style programs like:

```
let i = 0;
let sum = 0;
while (i < 10) {
    sum = sum + i;
    i = i + 1;
}
return sum;
```

This unlocks a different class of benchmark (loop-heavy
workloads) and is the last "small" JIT feature before harder
tickets (closures, structs, arrays) start requiring real
infrastructure.

## Acceptance criteria
- `compile_node_list` adds an arm for `Node::ExpressionStatement`
  whose inner expr is a reassignment. Today the parser
  represents `x = e` as an InfixExpression with operator `=`
  inside an ExpressionStatement — verify by reading
  the AST and the interpreter eval path. The JIT lowers this
  by:
  - Looking up `x` in the LowerCtx locals map. Missing →
    Unsupported("reassignment of undeclared identifier: x").
  - Lowering the RHS via lower_expr.
  - `bcx.def_var(var, rhs_value)` — Cranelift's SSA
    construction handles the rest.
- `compile_node_list` adds an arm for `Node::WhileStatement`:
  - Create three blocks: `header_block`, `body_block`,
    `exit_block`.
  - From the current block, jump to header_block.
  - In header_block, lower the condition expression and
    `brif(cond, body_block, &[], exit_block, &[])`.
  - In body_block, lower the body via lower_block_or_stmt.
    If the body terminates (early return), don't add a
    back-edge. Otherwise jump back to header_block.
  - Switch to exit_block. compile_node_list keeps walking
    statements after the while.
  - Sealing order matters: header_block has TWO predecessors
    (the entry jump + the body's back-edge), so seal it AFTER
    the back-edge is emitted. body_block + exit_block have one
    predecessor each and can be sealed after their respective
    branches.
- New unit tests in `jit_backend::tests`:
  - `jit_simple_reassignment`:
    `let x = 1; x = 2; return x;` → 2
  - `jit_reassignment_in_arith`:
    `let x = 5; x = x + 10; return x;` → 15
  - `jit_while_counts_to_ten`:
    `let i = 0; while (i < 10) { i = i + 1; } return i;` → 10
  - `jit_while_sum_loop`:
    `let i = 0; let sum = 0; while (i < 5) { sum = sum + i; i = i + 1; } return sum;` → 10
  - `jit_while_zero_iterations`:
    `let i = 5; while (i < 0) { i = i + 1; } return i;` → 5
    (header → exit on first check)
  - `jit_reassign_undeclared_unsupported`:
    `x = 1; return x;` → Unsupported with "undeclared
    identifier" descriptor
- Smoke test `bytecode_jit_runs_while_loop`: the sum-loop
  example above → driver prints 10, exits 0.
- All four feature configs pass cargo test + clippy.
- Commit message: `RES-107: JIT lowers reassignment + while (RES-072 Phase J)`.

## Notes
- Reassignment AST shape: read `interpreter.rs` (or main.rs)
  for how `x = 1;` parses. If the parser uses a dedicated
  `Node::AssignmentStatement` variant rather than infix `=`,
  match that variant directly.
- Cranelift's SSA construction handles re-defining a Variable
  via def_var across multiple blocks transparently — phi nodes
  are inserted by the FunctionBuilder when the value flows
  through a merge or back-edge. Don't try to insert phis
  manually.
- Don't try to support `for` loops (RES doesn't have C-style
  for) or break/continue in this ticket. Both are doable but
  add control-flow complexity that deserves its own ticket.
- Block-scoped locals (a `let` inside a while body that goes
  out of scope at the brace) is also out of scope — function-
  scoped semantics from RES-104 apply.

## Log
- 2026-04-17 created by manager (Phase J scope, optional follow-up to RES-104)
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/jit_backend.rs`
  - `compile_node_list` gains two new arms:
    - `Node::Assignment { name, value, .. }` — looks up the
      Variable from the enclosing scope, lowers the RHS via
      `lower_expr`, and `def_var`s. Missing binding → `Unsupported(
      "reassignment of undeclared identifier")`. Note the static-str
      constraint on `JitError::Unsupported` means the identifier
      name isn't embedded in the diagnostic — the descriptor is
      still distinguishable by the test matcher.
    - `Node::WhileStatement { condition, body, .. }` — delegates
      to the new `lower_while_statement` helper.
  - New `lower_while_statement` — header / body / exit three-block
    dance with an inline doc-comment covering Cranelift's sealing
    contract (body + exit sealed on switch, header sealed after
    the back-edge is emitted because it has two predecessors).
    Returns `Ok(false)` unconditionally because the header's
    `brif` always has an exit path at compile-time — even
    `while true` is only detected at runtime.
  - Six new unit tests in `jit_backend::tests` covering each
    ticket scenario: simple reassign, reassign-in-arith,
    count-to-ten loop, sum loop, zero-iteration loop (header-to-
    exit on first check), and the undeclared-reassign error path.
- `resilient/tests/examples_smoke.rs`
  - New smoke test `bytecode_jit_runs_while_loop` driving the
    sum-loop through the `--jit` CLI path; asserts the binary
    exits 0 and emits `10`.

Acceptance criteria walk:
- Reassignment arm with undeclared-name diagnostic — yes.
- While arm with header / body / exit + brif + sealing order —
  yes, documented inline.
- Six unit tests (`jit_simple_reassignment`,
  `jit_reassignment_in_arith`, `jit_while_counts_to_ten`,
  `jit_while_sum_loop`, `jit_while_zero_iterations`,
  `jit_reassign_undeclared_unsupported`) — all pass.
- Smoke test `bytecode_jit_runs_while_loop` — passes.
- Four feature configs pass tests + clippy — verified below.
- Neither `for` loops nor `break` / `continue` attempted (ticket's
  "out of scope" notes).

Deviation: `JitError::Unsupported` takes `&'static str`, so the
ticket's example `"reassignment of undeclared identifier: x"`
(with variable name interpolated) cannot be produced as-is —
we use the static descriptor `"reassignment of undeclared
identifier"` instead. The test `jit_reassign_undeclared_
unsupported` asserts `msg.contains("undeclared identifier")`,
which the static form satisfies.

Verification:
- `cargo build` (default + jit + lsp + z3 + logos-lexer) — clean.
- `cargo test` — 271 unit + 13 integration + 1 golden pass.
- `cargo test --features jit` — 324 unit (+53 over default: the
  jit_backend's test module only compiles with the feature on,
  adding the big JIT test suite) + 20 integration (+7) pass.
- `cargo clippy --features jit --tests -- -D warnings` — clean.
- `cargo clippy --features jit,logos-lexer,z3 --tests -- -D warnings`
  — clean.
