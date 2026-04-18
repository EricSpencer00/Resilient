---
id: RES-107
title: JIT lowers reassignment + while loops (RES-072 Phase J)
state: OPEN
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
