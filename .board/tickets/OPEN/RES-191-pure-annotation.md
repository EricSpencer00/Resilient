---
id: RES-191
title: `@pure` function annotation + purity checker
state: OPEN
priority: P3
goalpost: G18
created: 2026-04-17
owner: executor
---

## Summary
Mark functions as side-effect-free and have the checker enforce
it. First concrete G18 (effect tracking) ticket. A pure fn can
only:
- call other pure fns,
- read its parameters,
- do arithmetic / logic / comparison,
- construct / destructure values.

It can NOT: `println`, `file_*`, mutate captures, or call
unannotated user fns.

## Acceptance criteria
- Attribute syntax: `@pure\nfn name(...)`. Parser annotates the
  Function node.
- Checker: walks the body; any violation produces a diagnostic
  with the violating site's span and the reason ("calls unannotated fn foo"
  / "calls impure builtin println").
- Builtins tagged in the registry as pure / impure; the initial
  tag list goes into a table in `typechecker.rs`.
- Recursive purity: `@pure fn a() { b(); } @pure fn b() { a(); }`
  passes so long as neither does anything impure. Implementation:
  assume purity optimistically, verify, backtrack on violation.
- Unit tests covering: success, impure call, impure builtin, pure
  mutual recursion.
- Commit message: `RES-191: @pure annotation + checker`.

## Notes
- `@pure` is checked, not inferred — inference is RES-192.
- A future ticket makes the verifier trust @pure fns for SMT
  reasoning (currently it treats all fns as arbitrary).

## Log
- 2026-04-17 created by manager
