---
id: RES-154
title: Struct literal shorthand `Point { x, y }`
state: IN_PROGRESS
priority: P3
goalpost: G12
created: 2026-04-17
owner: executor
---

## Summary
`Point { x: x, y: y }` is tiresome when the local variable name
matches the field name. Teach the parser to accept the shorthand
`Point { x, y }` — desugars to the full form before the
typechecker sees it.

## Acceptance criteria
- Parser: in struct-literal field position, an identifier with no
  following `:` expands to `name: name`. Mixing with explicit
  `other: expr` in the same literal works.
- Error if the shorthand name isn't bound as a local: usual "unknown
  identifier" diagnostic (the desugared form produces it naturally).
- Unit tests: pure shorthand, mixed shorthand + explicit, unbound
  identifier error.
- SYNTAX.md "Structs" section gets a shorthand example.
- Commit message: `RES-154: struct-literal field shorthand`.

## Notes
- Desugaring happens in the parser, not a later pass — keeps the
  typechecker, interpreter, VM, and JIT ignorant of the sugar.
- Don't add field-punning to struct patterns yet — that's covered
  under RES-155 destructuring.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
