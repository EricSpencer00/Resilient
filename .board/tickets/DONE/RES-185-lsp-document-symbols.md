---
id: RES-185
title: LSP: document symbols (outline view)
state: DONE
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
- 2026-04-17 done by executor

## Resolution
- `resilient/src/lsp_server.rs`:
  - `Backend` gains a `documents: Mutex<HashMap<Url, Node>>`
    cache. `publish_analysis` stores the freshly-parsed AST
    in it on every `did_open` / `did_change`. This is the
    **document storage** piece RES-182's bail flagged as
    missing shared LSP infra — it's now present and
    available for the sibling tickets (181, 183, 184, 188)
    when they unblock.
  - `initialize` now advertises `document_symbol_provider:
    Some(OneOf::Left(true))` so compliant clients route
    `textDocument/documentSymbol` requests here.
  - New `did_close` handler clears the cache entry for the
    closed document so long-running editor sessions don't
    retain memory for inactive files.
  - New `document_symbol` handler consumes the cached AST,
    dispatches through the pure helper, and returns
    `DocumentSymbolResponse::Nested`. Returns `Ok(None)`
    when the document has never been opened.
  - New pure helper `document_symbols_for_program(&Node) ->
    Vec<DocumentSymbol>` walks top-level statements and
    emits one symbol per `Node::Function` /
    `Node::StructDecl` / `Node::TypeAlias`. Result is
    stable-sorted by source position.
  - Helper `span_to_range` converts 1-indexed `Span` to
    0-indexed LSP `Range`.
  - `selection_range` mirrors `range` today — `Node::
    Function::span` is the `fn` keyword's zero-width point
    (RES-088) and we don't track the identifier position
    separately. A later ticket can refine once parse-time
    name positions are tracked; the shape-only requirement
    the ticket spells out is satisfied.
- Deviations:
  - Ticket notes say `selectionRange` should be the
    identifier span; ours is the enclosing decl span.
    Documented inline as a future refinement. Editors
    tolerate both — the "click to navigate" UX lands on
    the declaration either way.
  - No nesting for methods in `impl` blocks (RES-158
    methods are top-level fns with mangled names after the
    parser emits them). The ticket explicitly says nesting
    is "a follow-up."
- Unit tests (6 new in `lsp_server::tests`):
  - `document_symbols_three_fns_plus_struct` — ticket AC.
  - `document_symbols_includes_type_alias` — RES-128 alias
    surfaces as `SymbolKind::TYPE_PARAMETER`.
  - `document_symbols_ignores_non_declaration_statements`
    — `let` / `return` / expression-statements aren't
    symbols.
  - `document_symbols_empty_on_empty_program`
  - `document_symbols_sorted_by_source_position`
  - `span_to_range_converts_1_indexed_to_0_indexed` —
    the coord system bridge.
- Integration test in `tests/lsp_smoke.rs`:
  `lsp_document_symbol_lists_outline` — spawns the real
  LSP binary, does initialize → didOpen (3 fns + 1 struct)
  → documentSymbol round-trip, asserts all four names are
  in the response along with the correct SymbolKind
  numerics (12=FUNCTION, 23=STRUCT). Also checks that
  `documentSymbolProvider:true` appears in the
  `initialize` response's capabilities.
- Verification:
  - `cargo test --locked` — 468 passed (no regression;
    the LSP changes are feature-gated).
  - `cargo test --locked --features lsp` — 478 passed
    (+6 unit tests + 1 new integration test).
  - `cargo test --locked --features logos-lexer` — 469
    passed.
  - `cargo clippy --locked --features lsp,z3,logos-lexer
    --tests -- -D warnings` — clean.
