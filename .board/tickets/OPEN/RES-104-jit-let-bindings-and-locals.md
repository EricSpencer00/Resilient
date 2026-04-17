---
id: RES-104
title: JIT lowers let bindings + identifier reads (RES-072 Phase G)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
After RES-102 (Phase E: if/else) and RES-103 (Phase F:
fallthrough), the JIT can compile programs with control flow
but no variables. Phase G adds `let` bindings + identifier
reads — the smallest step that lets the JIT compile programs
like:

```
let x = 5 + 10;
if (x > 10) { return x; } else { return 0; }
```

This is the foundation for compiling user-defined functions and
eventually `fib` itself (RES-105: function calls; RES-106: fib
benchmark).

## Acceptance criteria
- Locals storage: replace the current "single-block, no locals"
  shape with a per-function `HashMap<String, Variable>` (using
  cranelift's `Variable` type for SSA-friendly locals).
- `lower_expr` adds an arm for `Node::Identifier { name, .. }`:
  - Look up `Variable` in the local map; if missing return
    `Unsupported("identifier not in scope")`.
  - Emit `bcx.use_var(var)`.
- `compile_node_list` adds an arm for `Node::LetStatement { name, value, .. }`:
  - Lower the RHS via `lower_expr`.
  - Declare a fresh `Variable` (incrementing counter), call
    `bcx.declare_var(var, types::I64)`.
  - Insert into the local map: `locals.insert(name.clone(), var)`.
  - Call `bcx.def_var(var, rhs_value)`.
  - Continue to next statement (no terminator emitted).
- The locals map must be threaded through `compile_node_list`,
  `lower_expr`, `lower_if_statement`, and `lower_block_or_stmt`.
  Pass as `&mut HashMap` parameter; don't make it global.
- New unit tests in `jit_backend::tests`:
  - `jit_let_and_use`:
    `let x = 5; return x + 10;` → 15
  - `jit_let_in_arith`:
    `let a = 3; let b = 4; return a * b + 2;` → 14
  - `jit_let_in_if_condition`:
    `let x = 5; if (x > 0) { return x; } else { return 0; }` → 5
  - `jit_let_in_arm`:
    `if (1 < 2) { let y = 7; return y; } else { return 0; }` → 7
    (proves locals work inside arm blocks; the recursion
    through compile_node_list threads the map down)
  - `jit_undeclared_identifier_unsupported`:
    `return undefined_var;` → Unsupported("identifier not in scope")
- Update `jit_rejects_let_for_now` test from earlier phases —
  let bindings work now, retire or repurpose.
- Smoke test: `let x = 100; let y = 4; return x / y;` → 25.
- All four feature configs pass cargo test + clippy.
- Commit message: `RES-104: JIT lowers let + identifiers (RES-072 Phase G)`.

## Notes
- Cranelift `Variable` is just a `u32` newtype; you create them
  by incrementing a counter or via `Variable::with_u32`. The
  declare_var/def_var/use_var lifecycle gives Cranelift enough
  info to do SSA construction internally.
- DON'T try to handle reassignment (`x = x + 1`) in this ticket
  — that needs another `def_var` call with a non-trivial story
  about which value flows into which use. Keep this ticket
  immutable-only; reassignment is RES-107.
- Variable shadowing within a Block (`let x = 1; let x = 2;`)
  should work naturally — the second `let x` overwrites the
  HashMap entry, so subsequent uses get the fresh Variable.
  Add a test for that if it doesn't naturally drop out.
- Locals lifetime: in Phase G, locals are function-scoped (not
  block-scoped). Cranelift doesn't care about Rust-style block
  scoping for SSA — the Variable lives as long as the
  FunctionBuilder. If users need block-scoped locals later
  (RES-108), re-init the map at block entry.

## Log
- 2026-04-17 created by manager (Phase G scope)
