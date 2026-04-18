---
id: RES-181
title: LSP: hover shows inferred type of the symbol under cursor
state: OPEN
priority: P2
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
RES-074 scaffolded the LSP; RES-090/093/094 landed integration
tests. Hover is the most-used LSP feature and the easiest
high-signal extension: given a position, return the inferred type
of the expression/binding there.

## Acceptance criteria
- `Backend::hover` implementation in `lsp_server.rs`.
- On a position inside an identifier: walk the AST for the
  enclosing node; return `Hover { contents: MarkedString(type_str), range }`
  where `type_str` is the inferred type from RES-120's inferer or
  the typechecker's recorded type if RES-120 isn't enabled.
- On a position inside a literal: return the literal's type
  ("Int" for a number literal).
- No hover for whitespace / comments (null response).
- Integration test under `tests/lsp_hover.rs` spawns the binary,
  opens a document, sends `textDocument/hover`, asserts the
  expected type string on three positions.
- Commit message: `RES-181: LSP hover shows inferred type`.

## Notes
- Markdown is rendered by some clients, plain by others — use
  `MarkedString::String` to keep output universal.
- If inference failed for the fn, return the last known type
  rather than nothing — a partial answer is better than blank.

## Log
- 2026-04-17 created by manager
