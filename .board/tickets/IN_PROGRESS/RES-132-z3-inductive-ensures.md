---
id: RES-132
title: Z3 discharges simple inductive `ensures` via loop invariants
state: OPEN
priority: P2
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
`fn sum(Int n) requires n >= 0 ensures result >= 0 { let s = 0; for i in 0..=n { s = s + i; } return s; }`
is currently unprovable — the verifier has no notion of loop
invariants. Add explicit `invariant` annotations on `for`/`while`
and thread them into the SMT context as assume-at-entry / verify-at-back-edge
obligations.

## Acceptance criteria
- Parser: `while (c) invariant (p) { ... }` and
  `for x in xs invariant (p) { ... }`.
- Encoding:
  - Assume invariant on entry → verify it holds before the loop.
  - Inside the body, assume invariant + loop condition.
  - At back-edge, verify invariant still holds.
  - After the loop, assume invariant + negation of condition.
- Verifier-only feature; interpreter/VM/JIT ignore the annotation
  (it's a proof aid, not a runtime check).
- Unit tests: `sum(n) ensures result >= 0` with invariant `s >= 0`
  discharges; with no invariant, fails cleanly.
- Commit message: `RES-132: Z3 uses loop invariants to prove ensures`.

## Notes
- We deliberately do not infer invariants automatically — that's a
  research project. Users write them.
- The `--audit` table gains a "loop invariants" column.

## Log
- 2026-04-17 created by manager
