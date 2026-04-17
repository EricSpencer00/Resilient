---
id: RES-099
title: JIT lowers integer Sub/Mul/Div/Mod (RES-072 Phase C)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
RES-096 (Phase B) lowered IntegerLiteral + Add. This ticket
extends `lower_expr` in `jit_backend.rs` to handle the rest of
the integer infix operators: `-`, `*`, `/`, `%`. Same pattern as
RES-096 — recursive lower of left and right, then emit the
appropriate Cranelift instruction.

This is the smallest meaningful follow-up that grows the JIT's
expression surface. Control flow is RES-100; function calls and
the fib bench follow after that.

## Acceptance criteria
- `lower_expr` adds arms for `-`, `*`, `/`, `%`:
  - `-` → `bcx.ins().isub(l, r)`
  - `*` → `bcx.ins().imul(l, r)`
  - `/` → `bcx.ins().sdiv(l, r)` (signed integer divide)
  - `%` → `bcx.ins().srem(l, r)` (signed integer remainder)
- Anything still outside the supported subset returns
  `JitError::Unsupported("infix operator other than +,-,*,/,%")`
  with the descriptor updated to reflect the broader coverage.
- New unit tests in `jit_backend::tests`:
  - `jit_subtraction`: `return 10 - 3;` → 7
  - `jit_multiplication`: `return 6 * 7;` → 42
  - `jit_division`: `return 100 / 4;` → 25
  - `jit_modulo`: `return 17 % 5;` → 2
  - `jit_arith_chain`: `return (2 + 3) * 4 - 5;` → 15 (or pick
    a parenthesized expression that exercises precedence + all
    four ops). Note: the parser doesn't have explicit grouping
    in the JIT path's smoke shape; if `(...)` doesn't parse for
    `return EXPR;`, use a concrete program like
    `return 2 + 3 * 4;` (Pratt precedence already gives 14).
- Smoke test in `tests/examples_smoke.rs` (gated `--features jit`):
  writes a temp file with `return 100 / 4;`, runs `--jit`,
  asserts stdout contains `25` and exits 0.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` pass
  on all four feature configs.
- Commit message: `RES-099: JIT lowers Sub/Mul/Div/Mod (RES-072 Phase C)`.

## Notes
- Cranelift integer ops:
  - `iadd` (already used) → wrapping signed add
  - `isub` → wrapping signed sub
  - `imul` → wrapping signed mul
  - `sdiv` → signed integer divide (UB on rhs == 0; consider
    emitting a runtime check that returns a sentinel or trapping
    op — start simple and document the gap, since RES-091's VM
    line attribution doesn't yet apply to JIT'd code)
  - `srem` → signed integer remainder (same gap on rhs == 0)
- Don't add the `_op` parameter naming gymnastics from the prev
  variant — keep the same recursion shape as the existing `+`
  arm.
- Future ticket should add `JitError::DivideByZero` runtime check;
  for this ticket, just match the AST and emit. Note the gap in
  the commit body so it's discoverable.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
- 2026-04-17 executor: lower_expr extended with isub/imul/sdiv/srem;
  validate-first-then-recurse pattern short-circuits Unsupported
  before walking operands. Six new unit tests added in
  jit_backend::tests covering each op + a Pratt-precedence chain +
  a full four-op chain. The jit_rejects_subtraction_for_now test
  was repurposed → jit_rejects_comparison_for_now (subtraction
  works now; comparison ops are still the boundary). Smoke test
  bytecode_jit_runs_division added to tests/examples_smoke.rs:
  `return 100 / 4;` → driver prints 25, exits 0.
  Pre-existing clippy regressions from rust 1.91.0 fixed in
  passing (one approx_constant on a 3.14 lexer literal, two
  unused fn_span bindings in test code) so the matrix stays
  green. All four feature configs pass cargo test + clippy -D.
  Gap noted: sdiv/srem on rhs == 0 is UB at the IR level — a
  future ticket should emit a runtime check, since RES-091's VM
  line attribution doesn't apply on the JIT path yet.
