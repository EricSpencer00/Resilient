---
id: RES-184
title: LSP: rename symbol (prepareRename + rename)
state: OPEN
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Refactor basics: given a symbol under the cursor and a new name,
emit a workspace edit that renames every reference. Build on the
resolution table from RES-182/183.

## Acceptance criteria
- `Backend::prepareRename` returns the range of the symbol under
  the cursor if it's renamable (local, param, top-level fn).
  Returns null otherwise.
- `Backend::rename` returns a `WorkspaceEdit` grouping per-file
  `TextEdit` lists.
- New-name validation: must match the identifier pattern
  (`[A-Za-z_][A-Za-z0-9_]*`); else return an LSP error.
- Collision detection: if the new name shadows a still-visible
  binding, return an LSP error `rename would shadow <name>` rather
  than produce broken code.
- Integration test renaming a top-level fn + its callers, asserts
  every edit is present.
- Commit message: `RES-184: LSP rename symbol`.

## Notes
- Don't rename struct fields yet — separate ticket since it
  touches struct literal shorthand (RES-154) semantics.
- `prepareRename` is the UX guard — users get the "cannot rename
  here" feedback before they type.

## Log
- 2026-04-17 created by manager
