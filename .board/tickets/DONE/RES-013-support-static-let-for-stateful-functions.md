---
id: RES-013
title: Support `static let` for stateful functions
state: DONE
priority: P2
goalpost: G7
created: 2026-04-16
owner: executor
---

## Summary
The examples use `static let toggle = false;` inside functions to
persist state across calls (a poor man's module-level variable).
Today this is silently ignored / errors out because `static` is not a
recognized keyword and `let` inside a function only binds locally.

## Acceptance criteria
- `fn f() { static let x = 0; x = x + 1; return x; }` keeps `x` across
  calls to `f`
- `static` is a recognized keyword (`Token::Static`)
- Each function has a per-function persistent store populated on first
  entry and reused on subsequent calls
- Unit test: call a function with `static let counter = 0;
  counter = counter + 1;` three times and observe 1, 2, 3

## Notes
- Consider this the "MVP" of globals. A proper solution is top-level
  `static` or a module system; this ticket deliberately does not go
  that far.
- If implementation is larger than one ticket, split into lexer,
  parser, and runtime sub-tickets as they're discovered.

## Log
- 2026-04-16 created by manager
