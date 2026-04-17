---
id: RES-131
title: Z3 verifier proves array-index bounds in contracts
state: OPEN
priority: P2
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
RES-067 wired Z3; RES-068 elides runtime checks for fully-proven
fns. Neither touches array indexing. A function like
`fn head(Array<Int> xs) -> Int requires len(xs) > 0 { return xs[0]; }`
is provably safe but today still emits a runtime bounds check.
Teach the verifier to recognize `xs[i]` as generating a proof
obligation `0 <= i < len(xs)` and discharge it against the
precondition context.

## Acceptance criteria
- Verifier (SMT encoding side): `xs[i]` adds obligations
  `(>= i 0)` and `(< i (len xs))` where `len` is an uninterpreted
  function constrained by `>= 0`.
- Context: function `requires` predicates, enclosing branch
  conditions (already handled by RES-064), and `live`-block
  assumptions all flow in.
- If both obligations prove, RES-068's elision applies: no runtime
  bounds check at that site.
- `--audit` flag gains an "array bounds" row summarizing
  proven / unproven indexing sites per function.
- Unit + integration tests (tests/verifier_array_bounds.rs): two
  provable examples, two deliberately unprovable, one relying on
  a `requires` chain from a caller.
- Commit message: `RES-131: Z3 proves array-index bounds`.

## Notes
- `len(xs)` is a runtime-known value; model it as an uninterp fn
  `len :: Array -> Int` with axiom `>= 0`. We don't need to model
  the array contents for bounds proofs — just the length.
- If Z3 returns `unknown`, that's a failure to elide, not a
  verification failure — runtime check stays in.

## Log
- 2026-04-17 created by manager
