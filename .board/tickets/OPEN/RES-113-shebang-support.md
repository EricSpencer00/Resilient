---
id: RES-113
title: Shebang line `#!` at column 0 of line 1 is ignored by the lexer
state: OPEN
priority: P3
goalpost: G5
created: 2026-04-17
owner: executor
---

## Summary
`#!/usr/bin/env resilient` should let users make Resilient scripts
executable. The lexer today treats `#` as an unknown character. This
ticket teaches the lexer to skip a leading shebang when — and only
when — it appears at byte 0 of line 1.

## Acceptance criteria
- If `src.starts_with("#!")`, skip up to (and including) the next
  `\n` before tokenizing.
- Any `#!` elsewhere in the file is still a lex error — no free
  comment syntax from this change.
- Unit tests: `lexer_shebang_line_ignored`, `lexer_shebang_not_at_start_errors`,
  `lexer_empty_shebang_line` (`#!\n` then code).
- Golden: add `examples/shebang_demo.rs` starting with a shebang
  that prints "ok", matching an `.expected.txt` of `ok\n`.
- Commit message: `RES-113: ignore shebang line at start of file`.

## Notes
- Spans: the shebang bytes are consumed but not tokenized; the
  first real token's span still points at its actual byte offset
  (don't subtract the shebang length).
- The driver doesn't need to know about this — it's pure lexer
  concern.

## Log
- 2026-04-17 created by manager
