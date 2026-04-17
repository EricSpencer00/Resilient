---
id: RES-036
title: Live block invariants — closes G10
state: DONE
priority: P1
goalpost: G10
created: 2026-04-16
owner: executor
---

## Summary
Closes G10. Today `live { }` blocks retry on error but are blind
about what *state* is supposed to hold. Invariants let code say
"this block must always leave the system in a state where X is true"
— a system-level analogue of function contracts (RES-035).

## Acceptance criteria
- Syntax:

      live invariant fuel >= 0 {
          fuel = read_sensor();
      }

- Any number of `invariant EXPR` clauses, between `live` and `{`.
- Invariants are checked AT THE END of every successful iteration of
  the body. If any invariant is false, that iteration counts as a
  failure and triggers the existing retry logic.
- Invariant failure messages: `Invariant violation in live block: <expr> failed`.
- Existing live-block semantics unchanged when no invariants are
  declared.
- Tests:
  - live block with a passing invariant runs once, returns success
  - live block whose body violates an invariant retries and
    eventually fails with a contract-style error
  - multiple invariants all checked

## Notes
- Add Token::Invariant. Add `invariants: Vec<Node>` to Node::LiveBlock.
- eval_live_block already loops; insert an invariant-check pass after
  a successful body iteration.
- Share the format_contract_expr helper with RES-035 for error pretty-
  printing.

## Log
- 2026-04-16 created by manager
