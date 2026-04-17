---
id: RES-119
title: Unified `Diagnostic` type shared by parser, typechecker, verifier, LSP
state: OPEN
priority: P2
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
Parser errors, typechecker errors, verifier failures, and LSP
diagnostics are four similar-shaped structs right now, each with
their own code path for stringification. Consolidating behind a
single `Diagnostic { span, severity, code, message, notes }` makes
the LSP story straightforward and lets RES-206 hang error codes off
something real.

## Acceptance criteria
- New `resilient/src/diag.rs` (extending RES-117's module): define
  `struct Diagnostic { span: Span, severity: Severity, code: Option<DiagCode>, message: String, notes: Vec<(Span, String)> }`.
- `Severity` is `Error | Warning | Hint | Note`.
- `DiagCode` is a newtype around a small string like `"E0008"`;
  registry populated in RES-206.
- Parser, typechecker, interpreter, VM, and verifier error paths
  all return `Vec<Diagnostic>` instead of bespoke types.
- LSP `Backend::publish_diagnostics` consumes the unified type and
  converts to `tower_lsp::lsp_types::Diagnostic`.
- Rendering for terminal output delegates to
  `format_diagnostic_terminal` in `diag.rs`.
- All four feature configs pass cargo test + clippy.
- Commit message: `RES-119: unified Diagnostic across all phases`.

## Notes
- `notes` lets us attach a secondary span like
  `note: previous definition was here` — no new renderer work needed
  in this ticket; just provision the field.
- Don't try to refactor every callsite in one sweep if it blows up
  the diff — cover the compiler-side phases first, move LSP in a
  follow-up if needed. Favour a clean merge over strict one-ticket
  coverage.

## Log
- 2026-04-17 created by manager
