---
id: RES-112
title: `--dump-tokens` driver flag prints the token stream and exits
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/main.rs` — new `--dump-tokens` arg-parser arm +
  mutex check vs `--lsp` + short-circuit dispatch before the rest
  of the execute pipeline + a new `dump_tokens_to_stdout(src)`
  helper that drives `Lexer::new` / `next_token_with_span` through
  the default routing (hand-rolled or logos depending on feature
  flag) and prints `line:col  <Debug-of-Token>("<lexeme>")` one
  per line, terminating at `Eof`. The lexeme comes from the
  source via char-indexed slicing on `Span::{start.offset,
  end.offset}`, escaping `\n` / `"` / `\\` so multi-line strings
  stay on a single line in the output.
- `resilient/tests/dump_tokens_smoke.rs` (new) — three smoke tests:
  - `dump_tokens_prints_first_three_tokens_of_hello` asserts
    `Function("fn")` / `Identifier("main")` / `LeftParen` are the
    first three output lines on `examples/hello.rs`, and `Eof` is
    the last. The ticket sketch's `Fn` / `Ident` / `LParen` short
    names don't match the actual `Token` enum variants — the test
    asserts on the real names.
  - `dump_tokens_rejects_mutually_exclusive_lsp` — exit code 2 +
    "mutually exclusive" diagnostic when both flags are passed.
  - `dump_tokens_without_path_errors_cleanly` — exit code 2 +
    "requires a path" diagnostic when no file arg follows.
- `README.md` — new `### Debugging` subsection with a usage
  example under "Getting Started".

Verification:
- `cargo build --locked` — clean.
- `cargo test --locked` — 271 unit + 3 dump-tokens smoke + 12
  examples-smoke + 1 golden pass.
- `cargo clippy --locked -- -D warnings` — clean.
- `cargo clippy --locked --tests -- -D warnings` — clean.
- Manual: `resilient --dump-tokens examples/hello.rs` prints the
  expected stream (`2:1  Function("fn")` … `6:1  Eof("")`).

Deviation: the ticket sketch used short variant names
(`Fn` / `Ident` / `LParen`) that don't exist in the real codebase
(`Function` / `Identifier` / `LeftParen`). The integration test
asserts on the actual variant names.
