---
id: RES-235
title: "RES-133b: wire `assume()` predicates into the SMT/Z3 verifier context"
state: OPEN
priority: P2
goalpost: G9
created: 2026-04-20
owner: executor
---

## Summary

RES-133a shipped `assume(expr)` as a runtime-only construct (equivalent
to `assert` at runtime). The original RES-133 ticket specified that
`assume` should also feed its predicate into the SMT verifier as an
asserted fact for subsequent obligations in the same block — this half
was explicitly deferred as RES-133b in the commit message.

Without verifier threading, `assume(x > 0); ensures x > 0` forces the
verifier to re-prove what the user already declared safe, defeating the
purpose of `assume` in safety-critical code.

## Acceptance criteria

- In `resilient/src/verifier_z3.rs` (and the `--features z3` build
  path): when a `Node::Assume { condition, .. }` is encountered during
  SMT context generation, its predicate is emitted as an `(assert ...)`
  fact — identical to how `requires` clauses are added at function entry.
- Subsequent `ensures` / `assert` obligations in the same block may use
  the assume predicate as a given.
- Unit test (under `#[cfg(all(test, feature = "z3"))]`):
  `assume(x > 0); ensures x > 0` — the verifier proves the `ensures`
  without reporting an obligation.
- Unit test: `assume(false); ensures 1 == 2` — the verifier "proves" the
  `ensures` (because `false` implies anything); add a comment noting this
  is expected and is why `assume(false)` is dangerous (see RES-133c).
- `cargo test --features z3` remains fully green.
- `cargo clippy --all-targets -- -D warnings` remains clean.
- Commit message: `RES-235: wire assume() predicates into Z3 verifier context`.

## Notes

- The runtime behaviour from RES-133a is unchanged — `assume` still
  evaluates the predicate at runtime.
- Do **not** implement the `assume(false)` dead-code warning here — that
  is RES-236 (the former RES-133c).
- Do **not** add the `--audit` glyph here — that can be a follow-up once
  the verifier wiring is confirmed working.
- Do **not** modify existing tests — add only new ones.

## Dependencies

- RES-133a (shipped in commit 6ada8e3) — `Node::Assume` exists in the
  AST.

## Log
- 2026-04-20 created by analyzer (follow-up from RES-133a commit note)
