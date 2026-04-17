---
id: RES-110
title: logos lexer preserves source spans compatible with `span.rs`
state: DONE
priority: P2
goalpost: G5
created: 2026-04-17
owner: executor
---

## Summary
`span.rs` (RES-069) defines `Pos { line, col, byte }` and `Span { start,
end }`. The logos-based lexer from RES-108 gives us byte-range spans
natively but does not track line/col. Every downstream consumer
(typechecker diagnostics, LSP, parser errors) expects full `Pos`
metadata. This ticket walks the token stream and backfills line/col
from a pre-computed line-start index.

## Acceptance criteria
- `Lexer::build_line_table(src: &str) -> Vec<usize>` returns byte
  offsets of each line start (newline-terminated + EOF). O(n) single
  pass.
- `fn pos_from_byte(table: &[usize], byte: usize) -> Pos` — binary
  search, O(log n).
- Every token emitted by the logos lexer now carries a `Span`
  constructed via `pos_from_byte(start)` and `pos_from_byte(end)`.
- Unit tests covering: start of file, end of file, multi-byte UTF-8
  boundary, trailing-newline vs no-trailing-newline files.
- `lexer_parity` (from RES-108) extended to compare full `Span`
  values, not just lexemes.
- Commit message: `RES-110: logos tokens carry Pos-compatible spans`.

## Notes
- UTF-8: col is *character* count (grapheme clusters are overkill for
  source). Use `.chars().count()` on the slice from last newline to
  byte offset.
- Don't cache the line table across files — cheap enough to rebuild,
  and cross-file leakage is the first thing to bite us otherwise.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/main.rs`
  - Added `Lexer::build_line_table(src: &str) -> Vec<usize>` — the
    O(n) single-pass scanner that records the byte offset of each
    line start (BOF always at index 0, then each byte-after-`\n`).
  - Added free fn `pos_from_byte(&[usize], &str, usize) -> Pos` —
    O(log n) binary search into the table plus O(line-length)
    character counting for UTF-8-correct column and char-offset.
    The signature deviates from the ticket sketch
    (`(table, byte) -> Pos`) by including `src`; the accompanying
    UTF-8 note demanded access to the source text to count chars.
  - Added `last_token_offset: usize` field to `Lexer`, snapshotted
    in `next_token` alongside `last_token_line/column`, so the
    hand-rolled scanner no longer pins `start.offset = 0` in
    `next_token_with_span`.
  - Extended the `lexer_parity_on_all_examples` test (RES-108) to
    assert full `Span` equality — all of `(line, column, offset)`
    on both `start` and `end`.
  - Added eight targeted unit tests for `build_line_table` /
    `pos_from_byte`: empty source, no-newline source, multi-line,
    trailing newline, start-of-file, EOF without trailing newline,
    EOF with trailing newline, UTF-8 column boundary, and UTF-8
    across lines.
- `resilient/src/lexer_logos.rs`
  - Refactored `tokenize` to drop the bespoke `byte_to_pos` array
    and use `Lexer::build_line_table` + `pos_from_byte` instead.
  - Bumped the EOF sentinel to also advance `Pos::offset` by one
    (matching the hand-rolled lexer's trailing `read_char` bump).

Verification:
- `cargo build` → clean.
- `cargo build --features logos-lexer` → clean.
- `cargo test` → 226 unit + 13 integration pass (includes the eight
  new line-table / UTF-8 unit tests).
- `cargo test --features logos-lexer` → 227 unit (incl. parity +
  line-table) + 13 integration pass. Parity now compares full
  `Span` values, not just `(line, col)`.
- `cargo clippy --tests -- -D warnings` → clean.
- `cargo clippy --features logos-lexer --tests -- -D warnings` →
  clean.
