---
id: RES-074
title: Phase 5 LSP server scaffolding
state: DONE
priority: P2
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
G17 = Language Server Protocol. This ticket lands the SCAFFOLDING:
a separate `resilient-lsp` binary that speaks LSP over stdio, accepts
`initialize`, `textDocument/didOpen`, `textDocument/didChange`, and
publishes `textDocument/publishDiagnostics` containing parser + typechecker
errors. Hover, completion, go-to-definition come in follow-ups.

This is gated on RES-069 (Spans) — without spans we can't produce real LSP
ranges.

## Acceptance criteria
**Scoped-down approach**: put LSP code in `main.rs` behind the `lsp`
feature flag rather than a separate binary. Skips the lib/bin
refactor — we can revisit if we outgrow it.

- New `lsp` feature in `Cargo.toml` adding `tower-lsp = "0.20"` and
  `tokio = { version = "1", features = ["rt-multi-thread", "macros", "io-std"] }` as optional deps.
- Default builds do NOT pull in the LSP deps.
- New CLI flag `--lsp` on the `resilient` binary. Under
  `--features lsp`, it spins up the tower-lsp server on stdio; without
  the feature, it prints a helpful message and exits 1.
- New module `resilient/src/lsp_server.rs` (also gated on the `lsp`
  feature) implementing `tower_lsp::LanguageServer`:
  - `initialize` returns `TextDocumentSync::Full` capability.
  - `did_open` and `did_change` parse the buffer (via crate `parse()`),
    run the typechecker, and publish diagnostics via
    `client.publish_diagnostics(...)`. Each diagnostic's `Range` is
    derived from RES-077's per-statement spans.
  - `shutdown` is a no-op.
- Ranges use `tower_lsp::lsp_types::Position { line: span.start.line - 1, character: span.start.column - 1 }` so zero-indexed LSP values come out right.
- New `LSP.md` in the repo root documenting:
  1. Build with `cargo build --features lsp`.
  2. Editor config example (VS Code `settings.json` / Neovim snippet) pointing at the binary with `--lsp`.
- Smoke test: `cargo build --features lsp` succeeds (integration
  test that actually talks LSP over stdio is a follow-up — covered
  in a future ticket).
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` pass
  on default features (the LSP code is gated OUT).
- `cargo build --features lsp` and `cargo clippy --features lsp -- -D warnings` pass when the feature is on.
- Commit message: `RES-074: tower-lsp scaffolding publishes diagnostics (opt-in feature)`.

## Notes
- RES-077 already put spans on every top-level statement, so
  diagnostics from the typechecker (via `check_program_with_source`)
  can attribute to a specific line+column.
- Using `#[tokio::main]` at a dedicated `async fn lsp_main()`
  spawned via `tokio::runtime::Runtime::new().block_on(...)` means
  the non-LSP path stays fully synchronous. Alternatively wrap
  `main` in `tokio::main` under `#[cfg(feature = "lsp")]` — pick
  whichever is cleaner.
- Full LSP integration test (spawn binary, send `didOpen`, read
  diagnostics back) is a follow-up ticket; it requires LSP framing
  (`Content-Length: ...\r\n\r\n<json>`) which is real work.
- Stretch goal (NOT required): publish diagnostics for `requires`
  clauses that fail static verification.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager
- 2026-04-17 manager rescope: put LSP code in `main.rs` behind the
  `lsp` feature flag instead of a separate binary. Avoids the
  lib/bin refactor.
- 2026-04-17 executor landed:
  - Cargo.toml: new `lsp` feature; `tower-lsp = "0.20"` and
    `tokio = "1"` added as optional deps. Default build unchanged.
  - New `resilient/src/lsp_server.rs` (~175 lines) implementing
    `tower_lsp::LanguageServer`:
    - `initialize` returns `TextDocumentSync::Full` capability.
    - `did_open` and `did_change` route through
      `publish_analysis(uri, text)`: parse → typecheck → publish.
    - Typechecker errors from RES-080 come pre-formatted with
      `<uri>:<line>:<col>:` prefix — `extract_range_and_message`
      parses that back into an LSP `Range` and a clean message.
    - Parser errors land at 0:0 pending parser-span work.
    - `shutdown` is a no-op.
    - `run()` builds a tokio runtime and drives the Server on
      stdin/stdout.
  - New CLI flag `--lsp` in `main()`. Under `--features lsp` it
    dispatches to `lsp_server::run()`; without the feature it
    prints a helpful pointer and exits 1.
  - Module declared with `#[cfg(feature = "lsp")] mod lsp_server;`
    so no LSP code is compiled on the default path.
- 2026-04-17 tests:
  - 3 new unit tests in `lsp_server::tests` covering the
    range-extractor heuristic:
    - parses `<path>:<line>:<col>:` prefix
    - handles no-prefix gracefully (defaults to 0:0)
    - handles Windows-style paths with extra `:` characters
- 2026-04-17 `LSP.md` at repo root with build instructions and
  Neovim / VS Code editor config examples.
- 2026-04-17 verification across three feature configs:
  - default: 211 unit + 1 golden + 11 smoke = 223 tests
  - `--features z3`: 219 + 1 + 12 = 232
  - `--features lsp`: 214 + 1 + 11 = 226
  All three clippy `-- -D warnings` clean.
- Follow-ups (separate tickets when appropriate):
  - Parser-error position threading (currently all at 0:0)
  - Integration smoke test that spawns binary and exchanges LSP
    messages over Content-Length framing
  - Hover, completion, go-to-definition
