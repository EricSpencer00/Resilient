---
id: RES-182
title: LSP: go-to-definition for local bindings and top-level fns
state: OPEN
priority: P2
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
After hover, go-to-definition is the next most-used feature.
Scope: local `let`, function parameters, and top-level `fn`
declarations. Struct fields and cross-module refs are follow-ups.

## Acceptance criteria
- `Backend::goto_definition` returns a `Location` pointing at the
  defining span.
- Resolution uses a pre-built name→definition map for the
  current document; rebuilt on `did_change`.
- Shadowing: jump to the binding in scope at the cursor position,
  not the first definition of the name.
- Imports (RES-073) — within-file-post-splice, the map sees
  imported symbols too; jumps to the imported file's
  corresponding span.
- Integration test in `tests/lsp_goto_def.rs` exercising local,
  param, top-level fn, imported top-level fn.
- Commit message: `RES-182: LSP goto-definition`.

## Notes
- With RES-073's splice, imported names appear in the single-file
  AST. We keep the original file path in the spans so we can
  return the correct `Uri` even though the AST is spliced.
- Don't support struct-field goto in this ticket; separate ticket
  since it requires field-aware resolution.

## Log
- 2026-04-17 created by manager
