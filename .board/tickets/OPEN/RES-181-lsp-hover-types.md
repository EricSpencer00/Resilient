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
- 2026-04-17 claimed and bailed by executor (blocked on RES-120 +
  substantial new LSP infra)

## Attempt 1 failed

Two blockers.

1. **"Inferred type" source is RES-120**, which is bailed (OPEN
   with `## Clarification needed`). The fallback line in the
   acceptance criteria — "the typechecker's recorded type if
   RES-120 isn't enabled" — doesn't help: the typechecker today
   doesn't expose its environment after `check_program` and
   doesn't retain per-position type info for identifiers at all.
   Without one of those, hover has nothing but literal-type
   heuristics.
2. **New LSP infrastructure the current scaffolding doesn't
   carry**:
   - Document storage (`Arc<Mutex<HashMap<Url, String>>>` on
     `Backend`) — today `publish_analysis` receives text and
     drops it; hover requests have only a URI.
   - `did_close` handler to clean the map.
   - Capabilities advertisement (`hover_provider: Some(...)`).
   - AST position walk — find the deepest `Spanned<Node>` whose
     span contains the requested `(line, col)`.
   - `Backend::hover` implementation + return-shape conversion.
   - An end-to-end integration test in `tests/lsp_hover.rs` —
     ~120 lines mirroring `lsp_smoke.rs`'s framing pattern
     (initialize / didOpen / hover / shutdown).

## Clarification needed

Manager, please either:

- Gate RES-181 on RES-120 + a narrow ticket to expose the
  typechecker's top-level env / a per-identifier type table; or
- Rewrite as a literals-only hover (`Int` / `Float` / `Bool` /
  `String` on literal positions) and split identifier hover into
  RES-181b, deferred until RES-120 lands.

Option 2 is ~70% of the ticket's user value and would let the
shared scaffolding — document storage, capabilities, AST position
walk — earn its keep ahead of the other LSP tickets (completion,
go-to-def) that want the same plumbing.

No code changes landed — only the ticket state toggle and this
clarification note.
