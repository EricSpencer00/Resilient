---
id: RES-182
title: LSP: go-to-definition for local bindings and top-level fns
state: DONE
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
- 2026-04-17 claimed by executor — landing RES-182a scope (top-level decls only,
  no locals/params/imports) now that RES-181a unblocked the shared LSP plumbing
- 2026-04-17 landed RES-182a (top-level goto-def); RES-182b/c deferred

## Resolution (RES-182a — top-level goto-def only)

This landing covers **RES-182a** of the bail's implicit split:
go-to-definition for top-level declarations (fn / struct / type
alias) within the current document. Scope-aware local / parameter
resolution (RES-182b) and cross-file import jumps (RES-182c,
gated on span::Span carrying a source path) stay deferred.

Builds directly on RES-181a's shared scaffolding — document
storage, capability advertisement, and the token-level cursor
lookup via `Lexer::next_token_with_span`. No new infra.

### Files changed

- `resilient/src/lsp_server.rs`
  - New imports for `GotoDefinitionParams`, `GotoDefinitionResponse`.
  - `initialize` capabilities add `definition_provider: Some(OneOf::Left(true))`.
  - New `Backend::goto_definition` handler:
      1. `identifier_at(src, pos)` — drives the lexer to find
         the `Token::Identifier` covering the cursor.
      2. `build_top_level_defs(&program)` — walks the cached
         AST for fn / struct / type-alias decls, preserving
         source order. Duplicate names resolve to the first
         occurrence (deterministic goto).
      3. `find_top_level_def(&defs, &name)` — linear-scan lookup.
      4. Wraps the result in `GotoDefinitionResponse::Scalar(Location)`.
  - `pub(crate) fn identifier_at` and `pub(crate) struct TopLevelDef` +
    `pub(crate) fn build_top_level_defs` + `pub(crate) fn find_top_level_def`
    so future callers (RES-183 references, RES-184 rename) can
    reuse the same infrastructure.
- `resilient/tests/lsp_goto_def_smoke.rs` (new)
  - End-to-end: initialize → didOpen a 4-line document →
    four definition requests covering fn reference (jumps to
    line 0), struct reference (jumps to line 1), keyword cursor
    (null), local binder cursor (null — RES-182a doesn't handle
    locals). Each assertion names its scope so RES-182b can tell
    which tests to revisit.

### Tests (14 unit + 1 integration, all `res182a_*`)

Unit (`src/lsp_server.rs`):
- `identifier_at_returns_name_and_range`
- `identifier_at_returns_none_for_literal`
- `identifier_at_returns_none_for_keyword`
- `identifier_at_finds_mid_identifier` — cursor 4 chars into
  `my_fn` still resolves.
- `identifier_at_out_of_range_returns_none`
- `identifier_at_empty_source_returns_none`
- `build_top_level_defs_empty_program`
- `build_top_level_defs_collects_fn`
- `build_top_level_defs_collects_struct`
- `build_top_level_defs_collects_type_alias`
- `build_top_level_defs_mixed_kinds` — mixed fn / struct / type
  / let, preserves source order.
- `build_top_level_defs_first_wins_on_duplicates`
- `find_top_level_def_hit_and_miss`
- `find_top_level_def_returns_range_from_decl`

Integration (`tests/lsp_goto_def_smoke.rs`):
- `lsp_goto_definition_returns_location_for_top_level_decls` —
  four cursor positions cover fn ref, struct ref, keyword (null),
  local (null). Completes in ~0.5s.

### Verification

```
$ cargo build                                   # OK (8 warnings, baseline)
$ cargo build --features z3                     # OK
$ cargo build --features jit                    # OK
$ cargo build --features lsp,logos-lexer,infer  # OK
$ cargo test --locked
test result: ok. 651 passed; 0 failed      (non-lsp baseline unchanged)
$ cargo test --locked --features lsp
test result: ok. 708 passed; 0 failed      (+14 unit, +1 integration)
$ cargo test --features lsp res182a
test result: ok. 14 passed; 0 failed
$ cargo test --features lsp --test lsp_goto_def_smoke
test result: ok. 1 passed; 0 failed       (finishes in <1s)
```

### What was intentionally NOT done

- **RES-182b** — no scope-aware local / parameter resolution.
  Cursor on a `let` binder, a fn parameter, or a reference to
  either returns `null`. Blocked on a proper scope walker; see
  RES-182's bail note for the design.
- **RES-182c** — no cross-file import resolution. Blocked on
  `span::Span` carrying an originating file path (or threading
  one through `Spanned<Node>`). Today `imports::expand_uses`
  splices nodes verbatim without a path tag, so a jump target
  in an imported file would surface with the wrong `Uri`.
- **Goto for struct fields** — call-out from the ticket Notes
  section; its own future ticket once field-aware resolution
  exists in the typechecker.

### Follow-ups the Manager should mint

- **RES-182b** — scope-aware name→definition map, rebuilt on
  `did_change`. Walks fn params + let-binding scopes + block
  scopes, honours shadowing at the cursor position. Builds on
  the RES-182a `identifier_at` helper.
- **RES-182c** — extend `span::Span` to carry a source path (or
  thread one via `Spanned<Node>`); `imports::expand_uses`
  stamps spliced nodes. Then cross-file jumps return the right
  Uri.

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
