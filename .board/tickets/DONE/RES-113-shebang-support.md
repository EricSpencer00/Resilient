---
id: RES-113
title: Shebang line `#!` at column 0 of line 1 is ignored by the lexer
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/main.rs` — in `Lexer::new`'s legacy path, after
  the initial `read_char`, detect a leading `#!` and call
  `read_char` in a loop until `\n` / `\0`, then one more time to
  consume the `\n`. The lexer's existing line/col/position
  bookkeeping advances naturally, so the first real token's span
  points at its true byte offset (line 2, col 1 for a one-line
  shebang).
- `resilient/src/main.rs` — mirrored the same skip in the
  `tests::legacy_tokenize_with_spans` parity helper. Without
  this, the lexer-parity test diverges on inputs that start with
  `#!` (the logos path also skips shebangs; the helper manually
  constructs a Lexer and bypasses `Lexer::new`, so it needed its
  own skip).
- `resilient/src/main.rs` — four new unit tests:
  `lexer_shebang_line_ignored`, `lexer_shebang_not_at_start_errors`
  (asserts `Token::Unknown('#')` still fires mid-file),
  `lexer_empty_shebang_line` (`#!\n` + code),
  `lexer_shebang_only_no_trailing_newline`.
- `resilient/src/lexer_logos.rs` — in `tokenize`, detect `#!`
  prefix at byte 0 and compute `shebang_bytes = find('\n') + 1`
  (or `src.len()` if no newline). Feed logos only `&src[shebang_
  bytes..]`; offset every reported byte range by `shebang_bytes`
  when converting to `Pos` against the full-source line table.
  First real token's span therefore reports its true byte offset.
- `resilient/examples/shebang_demo.rs` + `.expected.txt` (new) —
  a `#!/usr/bin/env resilient` demo that prints `ok`; golden-
  backed so the example harness regression-tests it.

Verification:
- `cargo build --locked` — clean.
- `cargo test --locked` — 275 unit (+4 new) + 3 dump-tokens + 12
  examples-smoke + 1 golden (incl. the new `shebang_demo.rs`)
  pass.
- `cargo test --locked --features logos-lexer` — 276 unit
  (lexer_parity_on_all_examples now covers `shebang_demo.rs`
  too) + all integration pass.
- `cargo clippy --locked -- -D warnings` — clean.
- `cargo clippy --locked --features logos-lexer --tests -- -D warnings`
  — clean.
- `cargo clippy --locked --features z3,jit,logos-lexer --tests -- -D warnings`
  — clean.
- Manual: `resilient --dump-tokens examples/shebang_demo.rs`
  shows the first real token on line 2 col 1 (shebang not
  emitted). `#!` not at byte 0 still emits `Token::Unknown('#')`.
