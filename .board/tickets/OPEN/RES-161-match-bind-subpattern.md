---
id: RES-161
title: Bind-subpattern `case p @ Point { x, y } if x > 0 => ...`
state: OPEN
priority: P3
goalpost: G13
created: 2026-04-17
owner: executor
---

## Summary
The `name @ pattern` form lets the user bind the whole matched
value AND destructure it in the same arm. Useful for guards and
for forwarding the whole value to another function.

## Acceptance criteria
- Parser: `<name> @ <pattern>` at pattern position.
- Semantics: `name` binds the full value; inner pattern binds its
  parts. Both bindings are in scope in guards and arm bodies.
- Nested: `a @ Point { x: b @ Int, y }` — allowed but recursive
  only one level for this ticket (no `a @ (b @ ...)` chains).
- Unit tests covering bind-then-destructure with struct, tuple,
  and integer literal patterns; guard access to both `p` and
  `x`.
- Commit message: `RES-161: bind-subpattern @ in match`.

## Notes
- Type of `name` is the outer scrutinee's type; pattern bindings
  get their inferred types as usual.
- Mnemonic: the `@` reads as "also named" — document inline.

## Log
- 2026-04-17 created by manager
