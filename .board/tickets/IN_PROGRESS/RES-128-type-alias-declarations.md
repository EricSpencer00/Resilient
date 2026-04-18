---
id: RES-128
title: `type Meters = Int;` type alias declarations
state: OPEN
priority: P3
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
Aliases aren't new types — `Meters` unifies with `Int` — but they
document intent in function signatures and array shapes. Tiny
feature, immediate readability payoff.

## Acceptance criteria
- Parser: `type <Name> = <Type>;` at top level. No generics
  yet (`type Pair<A,B> = (A, B)` is RES-129's follow-up).
- Resolution: aliases expand eagerly at lookup time. A cycle (alias
  referring to itself transitively) is a diagnostic, not a panic.
- Unit test: `type M = Int; fn foo(M x) -> M { return x + 1; }`
  typechecks; `let m: M = "hi";` is a type error.
- SYNTAX.md gets a "Type aliases" subsection.
- Commit message: `RES-128: type alias declarations`.

## Notes
- Aliases do NOT create a nominal type — that's RES-126's
  territory. Document this inline with a `// alias is NOT
  nominal; use struct for newtype` comment.
- No forward references across modules yet; the resolver runs
  post-import-splice anyway (RES-073), so within-file forward refs
  are fine.

## Log
- 2026-04-17 created by manager
