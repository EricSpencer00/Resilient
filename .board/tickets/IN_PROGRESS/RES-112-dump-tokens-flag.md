---
id: RES-112
title: `--dump-tokens` driver flag prints the token stream and exits
state: OPEN
priority: P3
goalpost: G5
created: 2026-04-17
owner: executor
---

## Summary
Debugging lexer regressions today means adding `eprintln!` in
`Lexer::next_token` and reverting after. A driver flag that prints
`line:col  TOKEN(lexeme)` one per line and exits 0 makes the lexer
inspectable without code changes, and pairs with RES-108/110 parity
work.

## Acceptance criteria
- `cargo run -- --dump-tokens path/to/file.rs` prints to stdout in
  the format `L:C  Kind(\"lexeme\")` (one token per line, EOF on
  last line), exits 0.
- Works with both the hand-rolled and (when RES-108 is enabled) logos
  lexer — use the same driver routing as the normal run.
- Unit test: `tests/dump_tokens_smoke.rs` runs the binary on
  `examples/hello.rs` and asserts the first three tokens
  (`Fn`, `Ident("main")`, `LParen`).
- Documented in `README.md` under a new "Debugging" subsection
  (2-3 lines).
- Commit message: `RES-112: --dump-tokens driver flag`.

## Notes
- Existing flags (`--typecheck`, `--audit`, `--lsp`, etc.) go
  through a small arg-parse shim in `main.rs`; add the new flag
  there. Don't pull in `clap` for one more flag.
- Make it mutually exclusive with `--lsp` (no file arg when lsp).

## Log
- 2026-04-17 created by manager
