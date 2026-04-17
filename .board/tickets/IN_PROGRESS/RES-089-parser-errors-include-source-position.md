---
id: RES-089
title: LSP publishes parser errors at source line:col
state: OPEN
priority: P2
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
RES-074 (LSP scaffolding) noted as a follow-up that parser errors
all land at line 0, column 0 in published diagnostics because the
LSP backend only knows how to extract the typechecker's
`<path>:<line>:<col>:` prefix.

The hand-rolled parser ALREADY records errors with a `<line>:<col>:`
prefix (see `Parser::record_error` in `main.rs`). This ticket:
1. Generalizes `lsp_server::extract_range_and_message` to also
   recognize the bare `<line>:<col>:` form (no path prefix).
2. Routes parser errors through that extractor so they land at the
   correct LSP `Range` instead of 0:0.

Pure quality-of-life fix on top of the LSP. No new dependencies, no
AST changes, ~30 lines of code.

## Acceptance criteria
- `extract_range_and_message` (in `lsp_server.rs`) recognizes both
  forms:
  - `<path>:<line>:<col>: <message>` (typechecker; existing)
  - `<line>:<col>: <message>` (parser; new)
- Parser errors in `publish_analysis` are routed through
  `extract_range_and_message` instead of hardcoded `point_range(0, 0)`.
- New unit test in `lsp_server::tests`:
  - `extract_parses_bare_line_col_prefix`: input `"3:5: Unexpected token"` produces a `Range` at line 2, col 4 (0-indexed) and message `"Unexpected token"`.
- `cargo build --features lsp`, `cargo test --features lsp`,
  `cargo clippy --features lsp -- -D warnings` all pass.
- Default (no-feature) build untouched.
- Commit message: `RES-089: LSP routes parser errors through line:col extractor`.

## Notes
- `Parser::record_error` formats errors as
  `format!("{}:{}: {}", line, column, msg)`. The LSP just needs to
  parse that.
- The simplest extension: try the bare `<digit>:<digit>:` prefix
  first; if it doesn't match, fall back to the existing
  `<anything>:<digit>:<digit>:` heuristic; if neither matches,
  default to 0:0.
- Don't touch the typechecker — RES-080 already does the right thing.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
