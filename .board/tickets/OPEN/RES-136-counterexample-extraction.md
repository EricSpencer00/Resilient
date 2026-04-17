---
id: RES-136
title: Extract counterexamples from Z3 on verifier failure
state: OPEN
priority: P2
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
"Could not prove `ensures x > 0`" is useful; "could not prove
`ensures x > 0` — counterexample: `a = -1, b = 0`" is dramatically
more useful. When Z3 returns `sat` on the negation, pull the model
and print variable bindings.

## Acceptance criteria
- When the verifier calls the solver with the negated goal and gets
  `sat`, extract the model via `z3::Model::eval` for each
  declared free variable.
- Format: `counterexample: a = -1, b = 0` appended to the existing
  verifier error diagnostic.
- Variables with no assignment in the model are omitted (Z3 is
  free to not assign them).
- Only primitive values printed — BV values as decimal, arrays as
  summary ("len=N, ...") for now.
- New test `verifier_emits_counterexample` on a program that
  obviously fails.
- Commit message: `RES-136: counterexamples from Z3 models`.

## Notes
- If we're in `verifier-bv` mode (RES-134), print the decoded
  signed integer value, not the raw bitvector.
- Counterexamples are not formally part of the proof; just user
  aid. They should never be cached (RES-135).

## Log
- 2026-04-17 created by manager
