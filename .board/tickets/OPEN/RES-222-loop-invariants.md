---
id: RES-222
title: Loop invariants — `invariant expr;` inside `while` blocks
state: OPEN
priority: P2
goalpost: G9
created: 2026-04-20
owner: executor
---

## Summary
Add an `invariant` statement inside `while` loop bodies. The invariant is checked at loop entry and after each iteration at runtime, and encoded as an inductive assertion for Z3. Extends the existing `requires`/`ensures` contract system to loop reasoning.

## Acceptance criteria
- Parser accepts `invariant <expr>;` as a statement inside a `while` body. Placing `invariant` outside a loop is a parse error.
- At runtime the invariant is evaluated before the first iteration and after each iteration. A violation halts with `runtime error: loop invariant violated at line:col`.
- When Z3 is enabled, the verifier encodes the invariant as an inductive assertion (holds on entry given `requires`; preserved by one loop body execution).
- Multiple `invariant` statements in one loop body are all checked — treated as a conjunction.
- Golden test: `invariant_demo.rs` / `invariant_demo.expected.txt` — bounded counter loop with `invariant i >= 0 && i <= n`.
- Commit message: `RES-222: loop invariant statement with runtime check and Z3 inductive encoding`.

## Notes
- Start with runtime checking only; Z3 inductive path can be gated on `--features z3`.
- Prerequisite for RES-208 (concurrency actor loop safety).

## Log
- 2026-04-20 created by manager
</content>
</invoke>