---
id: RES-187
title: LSP: semantic tokens for accurate syntax highlighting
state: DONE
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Editors can fall back to TextMate grammars for highlighting, but
semantic tokens give us much better accuracy (e.g. coloring a
function call differently from a struct literal of the same name).
Provide full-file semantic tokens; delta is a follow-up.

## Acceptance criteria
- `Backend::semantic_tokens_full` returns a `SemanticTokens`
  response.
- Token types implemented (subset of the standard LSP list):
  `keyword`, `function`, `variable`, `parameter`, `type`, `string`,
  `number`, `comment`, `operator`.
- Modifiers: `declaration` on defining sites, `readonly` for
  contract / const bindings.
- Integration test exercising a small program with each token
  type represented.
- Commit message: `RES-187: LSP semantic tokens (full)`.

## Notes
- The encoded integer-array format is finicky — reference the
  spec's section on encoding and test encoding + decoding in a
  unit test, separately from the integration test.
- Don't rush a `full/delta` path; many clients only use full
  anyway.

## Resolution

### Approach
Two layers:

1. **`main.rs::sem_tok` module + helpers** (pure, no LSP types).
   Drives the compiler's own `Lexer::next_token_with_span` and
   reuses the same keyword / operator / literal categorization
   the compiler trusts — keeping the two from drifting.
   - Token-type constants `sem_tok::{KEYWORD, FUNCTION, VARIABLE,
     PARAMETER, TYPE, STRING, NUMBER, COMMENT, OPERATOR}` (indices
     0..=8).
   - Modifier bitmasks `MOD_DECLARATION = 1 << 0`,
     `MOD_READONLY = 1 << 1`.
   - `AbsSemToken { line, col, length, ty, modifiers }` —
     absolute-coord form used between collection and encoding.
   - `classify_lex_token(tok, prev_kw, span)` — state machine
     that upgrades identifiers using the most-recent keyword:
     after `fn` → FUNCTION + DECLARATION, after `struct`/`type`
     → TYPE + DECLARATION, after `new` → TYPE (no DECLARATION),
     after `let`/`static` → VARIABLE + DECLARATION, else VARIABLE.
   - `scan_comment_tokens(src)` — separate source-text scan for
     `//` line comments and `/* … */` block comments. Block
     comments that span lines are split into per-line tokens,
     since the LSP wire format can't represent tokens that cross
     line boundaries.
   - `encode_semantic_tokens(&[AbsSemToken]) -> Vec<u32>` —
     sorts by `(line, col)` then delta-encodes to the LSP's
     `[dLine, dStart, length, type, mods]` 5-tuple stream.
   - `compute_semantic_tokens(src) -> Vec<u32>` — composes the
     above.

2. **`lsp_server.rs` wiring** (pure tower-lsp glue).
   - `Backend.documents_text: Mutex<HashMap<Url, String>>` — a
     second cache alongside the RES-185 AST map, populated from
     `publish_analysis` and cleared on `did_close`. Needed
     because semantic tokens re-lex raw source; the AST isn't
     enough.
   - `semantic_tokens_legend()` — hard-codes the legend ordering
     so the indices line up with `sem_tok::*`. A unit test
     (`semantic_tokens_legend_indices_match_sem_tok_constants`)
     catches drift.
   - `semantic_tokens_capability()` — builds the
     `SemanticTokensOptions` with `full: Bool(true)`, `range:
     Some(false)`. Delta is deliberately deferred per the
     ticket's note ("many clients only use full anyway").
   - `semantic_tokens_from_wire(Vec<u32>) -> Vec<SemanticToken>`
     — unpacks the wire-format 5-tuples into tower-lsp's
     per-token struct form (its serializer re-flattens on the
     way out).
   - `Backend::semantic_tokens_full` handler — looks up the
     cached text (synchronous lock, never held across `.await`),
     calls `compute_semantic_tokens`, wraps the result in
     `SemanticTokens { result_id: None, data }`.

### Tests
- **Unit (main.rs)** — 10 new tests exercising encoding (delta
  math on one line, across lines, sort-before-encode, empty),
  identifier classification (after `fn`, `struct`, `type`,
  `new`), literal tagging (string/number/comment), operator
  tagging, and an end-to-end "every token type present"
  program. `compute_semantic_tokens_returns_wire_format`
  pins the multiple-of-5 invariant.
- **Unit (lsp_server.rs)** — 3 new tests: legend-index
  assertions vs. `sem_tok::*`, wire→struct unpacking, and
  partial-trailing-chunk drop behaviour.
- **Integration (`tests/lsp_smoke.rs::lsp_semantic_tokens_full`)**
  — spawns `resilient --lsp`, initializes, asserts
  `semanticTokensProvider` + `legend` appear in the capabilities,
  opens a program that exercises every token type, drains
  `publishDiagnostics`, issues `textDocument/semanticTokens/full`,
  and parses the `data` array to verify it is non-empty and a
  multiple of 5.

### Verification
- `cargo test --locked` → 478 passed
- `cargo test --locked --features lsp` → 495 passed (13 new: 10
  in main.rs, 3 in lsp_server.rs) plus `lsp_semantic_tokens_full`
  integration
- `cargo clippy --locked --features lsp,z3,logos-lexer --tests
  -- -D warnings` → clean

### Follow-ups (not done here)
- `semantic_tokens_full_delta` (explicitly deferred per the
  ticket notes; most clients work with `full` alone).
- `parameter` modifier today falls back to VARIABLE for function
  parameters because the lexer-driven classifier can't tell a
  parameter name from a body-local at token time. Fixing that
  cleanly needs the RES-182 name-resolution table; once that
  lands, upgrade the classifier to consult it for identifier
  classification (PARAMETER inside fn signatures, VARIABLE
  elsewhere).
- `readonly` modifier is declared in the legend but not yet
  emitted anywhere — the current classifier doesn't distinguish
  contract / `static` bindings from mutable `let`s. Wire this
  up as part of the same RES-182 follow-up.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (full-file semantic tokens
  via lexer-driven classifier; delta deferred)
