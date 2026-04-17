---
id: RES-100
title: JIT lowers comparisons and bool literals (RES-072 Phase D)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Phase C (RES-099) closed all four integer arithmetic ops. The
next building block on the road to JIT control flow (Phase E,
RES-102) is comparison ops + boolean literals at the expression
level. Lowering these in isolation keeps the change small and
reviewable, and gives the if/else ticket a foundation to stand
on.

The driver shape stays `return EXPR;`. After this ticket, the
JIT can compile programs like `return 5 < 3;` and the comparison
becomes a native `icmp` that returns an i64 0 or 1 — matching
how the bytecode VM materializes booleans.

## Acceptance criteria
- `lower_expr` adds arms for the six comparison operators:
  - `==` → `bcx.ins().icmp(IntCC::Equal, l, r)`
  - `!=` → `bcx.ins().icmp(IntCC::NotEqual, l, r)`
  - `<`  → `bcx.ins().icmp(IntCC::SignedLessThan, l, r)`
  - `<=` → `bcx.ins().icmp(IntCC::SignedLessThanOrEqual, l, r)`
  - `>`  → `bcx.ins().icmp(IntCC::SignedGreaterThan, l, r)`
  - `>=` → `bcx.ins().icmp(IntCC::SignedGreaterThanOrEqual, l, r)`
  - Cranelift's `icmp` returns an `i8` value. Use `uextend` to
    widen it to i64 so the function signature stays
    `extern "C" fn() -> i64`. (Cranelift will accept implicit
    width but the explicit extend keeps the lowering uniform.)
- `lower_expr` adds an arm for `Node::BooleanLiteral { value, .. }`:
  - `iconst(types::I64, if value { 1 } else { 0 })`
- The Unsupported descriptor for InfixExpression updates to:
  `"infix operator other than +,-,*,/,%,==,!=,<,<=,>,>="`
- New unit tests in `jit_backend::tests`:
  - `jit_lt_returns_zero_for_false`: `return 5 < 3;` → 0
  - `jit_lt_returns_one_for_true`: `return 3 < 5;` → 1
  - `jit_eq_int`: `return 7 == 7;` → 1; and a false case → 0
  - `jit_ne_int`: `return 1 != 2;` → 1
  - `jit_le_ge`: cover `<=` and `>=` boundary equality
  - `jit_bool_literal_true`: `return true;` → 1
  - `jit_bool_literal_false`: `return false;` → 0
  - `jit_compare_with_arith`: `return 2 + 3 < 10;` → 1 (proves
    composition with the existing arith lowerings)
- Update the previously-introduced `jit_rejects_comparison_for_now`
  test from RES-099 — comparison ops work now, so it should be
  retired or repurposed to a different unsupported-shape test
  (e.g. `return -x;` for prefix `-` or `return foo();` for call).
- Smoke test in `tests/examples_smoke.rs` (gated `--features jit`):
  `return 7 == 7;` → driver prints 1 and exits 0.
- `cargo test`, `cargo clippy --all-targets -- -D warnings` pass
  on default / z3 / lsp / jit feature configs.
- Commit message: `RES-100: JIT lowers comparisons + bool literals (RES-072 Phase D)`.

## Notes
- Watch for the `IntCC` import — it's `cranelift::prelude::IntCC`
  (re-exported from cranelift_codegen). The existing `use
  cranelift::prelude::*;` line should pick it up; if not, add an
  explicit import.
- The validate-first pattern from RES-099 should grow naturally:
  the matches! arm just appends the six new operators. Keep the
  recursive lower of left/right above the per-op match, same shape.
- `i8 → i64` extension: `bcx.ins().uextend(types::I64, raw_cmp)`.
  Don't use `sextend` — Cranelift's icmp result is unsigned (0 or 1
  in the low bit), and signed extend would still be correct, but
  uextend is the convention.
- Phase E (RES-102) will use the comparison results as branch
  conditions for `brif`. Don't try to ship `if/else` in this ticket
  — the cranelift block dance deserves its own change.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (Phase D scope)
- 2026-04-17 executor: lower_expr extended with the six comparison
  ops via `IntCC::*` → `icmp` → `uextend` to i64, plus
  `BooleanLiteral` → `iconst 0/1`. The Unsupported descriptor
  updated to list the full supported set. Eight new unit tests
  cover all six comparison ops (true + false branches), bool
  literals (true + false), <= / >= boundary equality, and a
  composition with arith (`2 + 3 < 10` → 1) that proves
  comparison can sit on top of the RES-099 lowerings.
  Updated jit_rejects_comparison_for_now → jit_rejects_prefix_for_now;
  comparisons work, prefix `-` is now the closest unsupported
  shape. Smoke test `bytecode_jit_runs_comparison` added:
  `return 7 == 7;` → driver prints 1, exits 0. Foundation in
  place for RES-102 (if/else needs an i64 0/1 condition feeding
  brif, which is exactly what this ticket produces). Matrix:
  default 217, z3 225, lsp 221, jit 238 — all green; clippy
  clean across all four configs.
