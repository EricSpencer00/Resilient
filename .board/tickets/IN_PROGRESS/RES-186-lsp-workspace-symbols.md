---
id: RES-186
title: LSP: workspace-symbol search across all project files
state: IN_PROGRESS
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Extends RES-185 to the whole workspace. The backend tracks all
open documents plus a one-time scan of any `.rs` files in the
workspace root at init. Returns a filtered list by substring
match.

## Acceptance criteria
- On `initialize`, walk the workspace root for `*.rs` files and
  pre-index top-level fns / structs / aliases. Watcher not
  required — refresh on `did_save` is enough.
- `Backend::workspace_symbol` returns up to 50 matching
  `SymbolInformation` entries, substring-match (case-insensitive)
  on the name.
- Integration test: pre-seed two files, invoke the query,
  assert both files' symbols are returned.
- Commit message: `RES-186: LSP workspace symbols`.

## Notes
- Don't respect `.gitignore` yet; small workspaces don't need it
  and respecting it requires a new dep.
- Index is held in memory; rebuilt per `did_save`. No persistence.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
