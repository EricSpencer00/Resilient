---
id: RES-223
title: SMT counterexample display — print Z3 witness on refutation
state: DONE
priority: P2
goalpost: G9
created: 2026-04-20
owner: executor
Claimed-by: Claude Sonnet 4.6
---

## Summary
When Z3 refutes a postcondition, print the concrete counterexample model so the user knows which input values break their contract. Currently the compiler only says "postcondition could not be verified" with no witness.

## Acceptance criteria
- When Z3 returns `Sat` (counterexample found), extract the model and format it as:
  ```
  error[E0042]: postcondition `ensures <expr>` violated
    --> file.rs:10:3
  counterexample:
    x = -1
    y = 0
  ```
- Each variable in scope at the `ensures` site that appears in the model is printed with its Z3-assigned value.
- When Z3 returns `Unsat` (proof succeeds) or `Unknown`, existing behavior is unchanged.
- Only print variables appearing in the `ensures` expression, not internal temporaries.
- Negative numbers print with a minus sign, not Z3's bitvector repr.
- Unit test: a known-bad function (`ensures result > 0` but can return 0) produces a model line in diagnostic output.
- Commit message: `RES-223: print Z3 counterexample witness on postcondition refutation`.

## Notes
- Use `z3::Model` to iterate over constants and extract integer/bool values.
- Requires `--features z3`.

## Log
- 2026-04-20 created by manager
- 2026-04-20 closed by Claude Sonnet 4.6 — commit 1bcd7c0
</content>
</invoke>