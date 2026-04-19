---
id: RES-188
title: LSP: completion for builtins and in-scope locals
state: IN_PROGRESS
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
- 2026-04-17 claimed and bailed by executor (same shared LSP infra
  gap flagged by RES-181 / RES-182)
- 2026-04-17 claimed by executor — landing RES-188a scope (builtins + top-level
  decls; locals/params deferred to RES-188b — same scope-walker gap as RES-182b)

## Attempt 1 failed

Same infra gap the last two LSP tickets flagged:

- `Backend` has no document storage today (`publish_analysis`
  receives text and drops it).
- `initialize` advertises neither `hover_provider`,
  `definition_provider`, nor `completion_provider`.
- No AST position walk.
- No `did_close` handler.

Completion-specific additions on top of the shared gap:

- Scope-aware collection of in-scope bindings at a given cursor
  position — the same walker RES-182 needs, plus a "keep bindings
  visible at this position" filter (stop descending once the
  position is passed).
- A `BUILTINS` iterator exported from `main.rs` for the LSP to
  enumerate builtin names + types. Currently `const BUILTINS: &[(&str,
  BuiltinFn)]` is `main.rs`-local and not pub.
- `Backend::completion` + 100-item cap + integration test in
  `tests/lsp_completion.rs`.

## Clarification needed

Recommend landing a shared-LSP-infra ticket first (see RES-181 and
RES-182's `## Clarification needed` sections, which propose the
same RES-XXX-a split). Once that's in, RES-188 reduces to: scope
walker + builtin enumeration + completion handler + test — one
iteration.

No code changes landed — only the ticket state toggle and this
clarification note.
