---
id: RES-110
title: logos lexer preserves source spans compatible with `span.rs`
state: OPEN
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
