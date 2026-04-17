---
id: RES-035
title: Function contracts (`requires` / `ensures`)
state: DONE
priority: P1
goalpost: G8
created: 2026-04-16
owner: executor
---

## Summary
Closes G8. Move Resilient one step toward its "formally verifiable"
tagline by letting functions declare pre- and post-conditions that are
checked at runtime. This is the MVP form of contracts; G9 (symbolic
verification) will re-check them at compile time.

## Acceptance criteria
- New syntax:

      fn divide(int a, int b)
          requires b != 0
          ensures result >= 0 || result < 0  // tautology, but demos binding
      {
          return a / b;
      }

- Parser accepts zero or more `requires` and zero or more `ensures`
  clauses between the parameter list and the body.
- Each clause is a boolean expression. The special identifier `result`
  in an `ensures` clause refers to the function's return value.
- Runtime: pre-conditions checked on entry (pre-arg-binding OK since
  they can only reference parameters). Violation → error of the form
  `Contract violation in fn <name>: requires <expr> failed`.
- Post-conditions checked after the function returns a value. `result`
  is bound to the return value in the clause's env.
- Clauses parse into new AST nodes; interpreter's apply_function
  runs them in order.
- Unit tests: valid pre, violated pre, valid post, violated post,
  result-binding.

## Notes
- This is the most strategic ticket on the board — it's the bridge
  from "scripting language" to "verifiable language." The assertions
  we add here become the proof obligations in G9.
- Don't over-engineer: one-line expressions only, no full predicate
  sublanguage. Extend later.
- Lexer: new keywords `requires`, `ensures`, `result`. (`result` can
  just be an identifier that happens to be bound; no token needed.)

## Log
- 2026-04-16 created by manager
