---
id: RES-185
title: LSP: document symbols (outline view)
state: IN_PROGRESS
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Populates the editor's outline view. Flat list of top-level fns,
structs, and type aliases with their spans. No nesting for
methods yet (RES-158 isn't landed when this is queued); add that
as a follow-up.

## Acceptance criteria
- `Backend::document_symbol` returns `DocumentSymbolResponse::Nested`
  (a vec of `DocumentSymbol`).
- For each top-level `fn`: name, kind=Function, range, selectionRange.
- For each `struct`: name, kind=Struct, range, selectionRange.
- For each `type` alias (post RES-128): name, kind=TypeParameter,
  range, selectionRange.
- Integration test opens a 3-fn + 1-struct document, asserts the
  response lists all four symbols.
- Commit message: `RES-185: LSP document symbols`.

## Notes
- `selectionRange` is just the identifier span; `range` is the
  whole decl span. Most clients use `selectionRange` for the "click
  to navigate" target and `range` for the enclosing box.
- Keep the response stable-sorted by source position so editors
  present it in file order.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
