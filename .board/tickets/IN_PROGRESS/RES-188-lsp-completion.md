---
id: RES-188
title: LSP: completion for builtins and in-scope locals
state: OPEN
priority: P2
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Minimum-viable completion: when the user types an identifier
prefix, offer builtins and in-scope bindings that start with that
prefix. No fuzzy matching yet; no type-driven filtering.

## Acceptance criteria
- `Backend::completion` returns a `CompletionResponse::Array` of
  `CompletionItem`.
- Source set: every builtin name (from the registry) + every
  in-scope local/param/top-level fn at the cursor position.
- Each item includes `kind` (Function / Variable / Keyword),
  `detail` (the item's type), and `insertText` (the name).
- Triggered by Ctrl-Space and by identifier-prefix typing (the
  client drives that — we just need to return responses promptly).
- Integration test exercising prefix completion inside a fn body.
- Commit message: `RES-188: LSP identifier completion`.

## Notes
- No post-dot completion yet — `.` for field access is a separate
  ticket once RES-185 / RES-155 settle.
- Limit results to 100 items — editors often truncate anyway,
  and large lists hurt latency.

## Log
- 2026-04-17 created by manager
