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
- 2026-04-17 claimed and bailed by executor (oversized; wants the
  same LSP infra RES-181 bailed on, plus a scope-aware resolver)

## Attempt 1 failed

This ticket sits on top of the LSP infrastructure gap RES-181
already flagged (no document storage on `Backend`, no hover / goto
capabilities advertised, no AST position walk) PLUS its own name-
resolution pass:

- **Scope-aware name → definition map**, rebuilt on `did_change`,
  honouring shadowing at the cursor position. That's a real
  walker: enter fn param scope → block scope → let-binding scope,
  tracking insertion order and handling re-binding.
- **Import resolution**: RES-073's splice flattens imports into a
  single AST, but the ticket requires spans to carry original file
  paths so `Location`'s `Uri` points at the right file. Today's
  `span::Span` doesn't carry a `source: PathBuf` / `Uri` field —
  `imports.rs` splices AST nodes verbatim, whose spans are in the
  imported file's coordinate system but without a path tag.
- **`Backend::goto_definition` + capabilities + integration test**
  in `tests/lsp_goto_def.rs` — mirrors the infra RES-181 needs.

## Clarification needed

Manager, please sequence this behind shared LSP plumbing:

- RES-XXX-a (new): LSP document storage + `did_close` + capabilities
  advertisement (`hover_provider` / `definition_provider`) +
  position-walk helper. Shared by RES-181, RES-182, RES-188.
- RES-XXX-b (new): extend `span::Span` to carry a source `PathBuf`
  (or thread one via `Spanned<Node>`); `imports::expand_uses`
  stamps the imported file onto spliced nodes' spans.
- Then RES-182 reduces to: scope-aware resolver + `goto_definition`
  handler + integration test — an iteration-sized ticket.

No code changes landed — only the ticket state toggle and this
clarification note.
