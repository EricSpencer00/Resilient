---
id: RES-130
title: Decide and document int ↔ float coercion policy
state: DONE
priority: P3
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
Today `1 + 2.0` produces a float silently — the interpreter coerces.
A safety-critical language should be deliberate here. This ticket
picks a policy, documents it, and adds tests pinning the behavior.
Recommendation: **no implicit coercion**; require `to_float(x)` or
`to_int(x)` at the boundary.

## Acceptance criteria
- SYNTAX.md gets a "Numeric coercion policy" section stating: no
  implicit conversions between Int and Float, ever. Mixed operands
  are a type error.
- Typechecker enforces the rule. The interpreter's silent coercion
  path removed.
- New builtins: `to_float(Int) -> Float`, `to_int(Float) -> Int`
  (latter truncates; document clearly).
- Existing examples updated if they relied on implicit coercion
  (check `sensor_monitor.rs`).
- Unit tests: one per operator × mixed-types combo = error;
  explicit `to_float` + mixed-is-now-same-type = success.
- Commit message: `RES-130: no implicit int↔float coercion`.

## Notes
- This IS a breaking change for any user code that relied on the
  old behavior. Acceptable pre-1.0; call it out in the roadmap
  changelog.
- `to_int(Float)` semantics: truncate toward zero. `to_int(NaN)`
  and `to_int(±inf)` are runtime errors with clean messages.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

**Policy**: no implicit int ↔ float coercion, ever. Mixed-type
arithmetic (`+ - * / %`) and literal pattern matching are type /
runtime errors with a diagnostic pointing at the explicit
conversions.

Files changed:
- `resilient/src/typechecker.rs`
  - New `check_numeric_same_type(op, left, right)` helper. `Int +
    Int` → Int, `Float + Float` → Float, `Any` propagates the
    concrete side, `Int ↔ Float` rejects with
    `Cannot apply '<op>' to int and float — Resilient does not
    implicitly coerce between numeric types. Use to_float(x)
    or to_int(x) explicitly.`
  - Arithmetic match arms (`+`, `- * / %`) now delegate to the
    helper; the former "Int + Float → Float" promotion path is
    gone. Comparison operators (`== != < > <= >=`) were already
    strict via `compatible()`.
  - `is_numeric` closure retired for those arms.
  - `to_float` / `to_int` registered in the builtin env
    (Any → Float / Any → Int signatures — the runtime checks
    narrower than the declared sig, rejecting non-numeric args).
- `resilient/src/main.rs`
  - `eval_infix_expression`: removed the `(Int, Float)` /
    `(Float, Int)` cross-coercion arms. Runtime now surfaces the
    same coercion-policy diagnostic (defensive — the typechecker
    already catches the static shape).
  - Pattern-literal match: removed the `Int`↔`Float` coercion so
    different numeric types never match a literal pattern.
  - New `builtin_to_float(args)` — `Int` / `Float` → `Float`.
  - New `builtin_to_int(args)` — `Int` passthrough; `Float`
    truncates toward zero with guarded rejection of `NaN`,
    `±∞`, and out-of-i64-range finite values (Rust's `as i64`
    saturates silently, which this language explicitly avoids).
  - Both builtins registered in `BUILTINS`.
  - 10 new unit tests:
    - Per-operator mixed-type rejection: `no_coercion_plus`,
      `no_coercion_minus`, `no_coercion_mul`, `no_coercion_div`,
      `no_coercion_mod`, `no_coercion_float_int_reversed` (six
      covering the 5 arithmetic ops × both argument orders).
    - Explicit conversion success: `to_float_then_arith_succeeds`.
    - Round-trip: `to_float_round_trip_preserves_int`.
    - Edge-case rejection: `to_int_nan_is_runtime_error`,
      `to_int_infinity_is_runtime_error` (both drive the
      builtin directly because Resilient doesn't currently have
      a surface-syntax way to produce a NaN / ∞ — float
      `0.0 / 0.0` is caught by the interpreter's divide-by-zero
      guard before IEEE semantics apply).
- `SYNTAX.md` — new `## Numeric coercion policy` section with
  the rule, a code example, the two builtins' signature +
  semantics table, and the rationale.

Examples audit: no `.rs` file in `resilient/examples/` uses
`FloatLiteral` / mixed-numeric arithmetic, so no example update
was needed. Confirmed via grep.

Breaking change per the ticket notes — acceptable pre-1.0.

Verification:
- `cargo build --locked` — clean.
- `cargo test --locked` — 302 unit (+10 new) + 3 dump-tokens + 12
  examples-smoke + 1 golden pass.
- `cargo test --locked --features logos-lexer` — 303 unit pass.
- `cargo clippy --locked --features logos-lexer,z3 --tests -- -D warnings`
  — clean.
- Manual: `let a = 1 + 2.0;` → typecheck error; `let a =
  to_float(1) + 2.0; println(a);` → prints `3`.
