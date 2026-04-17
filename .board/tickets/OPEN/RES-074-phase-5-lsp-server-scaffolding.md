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
- New binary target `resilient-lsp` declared in `Cargo.toml` (`[[bin]]`).
- Uses the `tower-lsp` crate (most ergonomic option).
- On `didOpen` and `didChange`, runs the existing parser + typechecker on
  the buffer text and publishes diagnostics with correct ranges derived
  from each error's `Span`.
- Manual smoke test (documented in a new `LSP.md`): editor connects, types
  a `let` with a type mismatch, sees a red squiggle on the offending range.
- Integration test in `resilient/tests/lsp_smoke.rs` that spawns the binary,
  sends a hand-rolled `didOpen` with an obviously-bad program, reads back
  the published diagnostics, and asserts at least one diagnostic with
  range matching the bad token.
- `cargo build` and `cargo test` all pass.
- Commit message: `RES-074: tower-lsp scaffolding publishes diagnostics`.

## Notes
- Blocked on RES-069 (Spans).
- Keep the LSP binary lean — it only depends on the parser + typechecker
  modules, never on the interpreter. May require a small refactor to
  expose those as a library crate; if so, do that refactor here.
- Stretch goal (NOT required for this ticket): publish diagnostics for
  `requires` clauses that fail static verification.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager
