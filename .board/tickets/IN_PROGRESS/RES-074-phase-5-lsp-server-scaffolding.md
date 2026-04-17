---
id: RES-074
title: Phase 5 LSP server scaffolding
state: OPEN
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
