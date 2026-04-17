---
id: RES-183
title: LSP: find-references for top-level functions
state: OPEN
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Counterpart to RES-182: given a cursor on a fn name, list every
location that calls it. Scope to top-level fns in the open file
(+ spliced imports). Local / param refs are less useful and out
of scope.

## Acceptance criteria
- `Backend::references` returns an array of `Location` covering
  every call site.
- Match is AST-driven, not textual — `Node::Call` with callee
  name equal to the target.
- `includeDeclaration: true` in the request adds the defining
  site; false omits it.
- Integration test with a 3-caller setup + a struct literal that
  uses the same name but is distinct (should not appear).
- Commit message: `RES-183: LSP find-references`.

## Notes
- Same name-resolution table from RES-182 is reused — no new
  pre-pass.
- Performance: linear scan of the AST is fine. Don't premature-
  optimize; typical files are small.

## Log
- 2026-04-17 created by manager
