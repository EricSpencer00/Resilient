---
id: RES-065
title: Use caller requires as assumptions inside the body
state: DONE
priority: P0
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
**Fifth Phase 4 brick.** When checking a function body, the
function's own `requires` clauses are KNOWN to hold for the
parameters. Today the verifier proves this at *call sites* (RES-061),
but inside the body it forgets. This ticket fixes that: the body is
checked with the requires clauses as assumptions in scope.

The result: contracts chain through call boundaries.

  fn pos(int x) requires x > 0 { return x; }

  fn caller(int n) requires n == 5 {
      // Inside here, the verifier knows n == 5.
      // pos(n) is then a call where x = 5; pos requires x > 0 → 5 > 0 ✓
      let r = pos(n);
  }

## Acceptance criteria
- A caller with `requires param == LIT` discharges interior calls
  whose contracts hold under that binding.
- A caller with NO requires still works (no regression).
- A caller with `requires` that contradicts an interior contract is
  rejected at compile time (catches design bugs early).
- Tests: 4+ covering the above.

## Notes
- Extends `extract_eq_assumption` — extract from each requires clause
  the same way we extract from if-conditions.
- Extends Function body check: before swapping env, push every
  extractable assumption into const_bindings. After checking, restore.
- ensures clauses are NOT pushed (they apply to result, not params).

## Log
- 2026-04-17 created and claimed
