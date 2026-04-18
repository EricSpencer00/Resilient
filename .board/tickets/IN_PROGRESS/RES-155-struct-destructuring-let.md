---
id: RES-155
title: Struct destructuring `let Point { x, y } = p`
state: IN_PROGRESS
priority: P3
goalpost: G12
created: 2026-04-17
owner: executor
---

## Summary
Destructuring a struct into local bindings is a common read
pattern. Pair with RES-154's shorthand so the same `{ x, y }` works
on both sides.

## Acceptance criteria
- Parser: `let <StructName> { field1, field2: local_name, .. } =
  expr;`. The `..` rest pattern allows ignoring trailing fields.
- Fields listed without `: name` bind to a local of the same name;
  with `: name` bind to an explicitly-renamed local.
- Exhaustiveness: without `..`, every field of the struct must
  appear — a typecheck error otherwise, listing missing fields.
- Unit tests: full destructure, renaming, rest pattern,
  non-exhaustive without `..`.
- Commit message: `RES-155: struct destructuring let`.

## Notes
- This is purely a let-binding feature — match arms get struct
  destructuring via RES-161.
- Don't support reference patterns / nested struct patterns in
  this ticket: one layer deep is enough to unblock most ergonomic
  wins. Deeper nesting is a follow-up.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
