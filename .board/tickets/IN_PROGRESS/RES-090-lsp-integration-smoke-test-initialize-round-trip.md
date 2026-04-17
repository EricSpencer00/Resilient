---
id: RES-090
title: LSP integration smoke test — initialize round trip
state: OPEN
priority: P2
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
RES-074 deferred a real LSP integration test. This ticket lands the
minimum useful one: spawn `resilient --lsp` as a subprocess, send a
hand-rolled `initialize` request over its stdin using LSP framing
(`Content-Length: N\r\n\r\n<json>`), read the response from its
stdout, and assert it's a well-formed JSON-RPC response that
includes server capabilities.

This proves the LSP binary is functional end-to-end, not just that
its module compiles. The full "send didOpen, read publishDiagnostics
notification, assert range" flow is a richer follow-up — this
ticket covers the foundational handshake.

## Acceptance criteria
- New file `resilient/tests/lsp_smoke.rs` (gated `#[cfg(feature = "lsp")]`).
- Test `lsp_initialize_round_trip`:
  1. Spawns the `resilient` binary with `--lsp` flag.
  2. Writes a valid LSP `initialize` request to its stdin using
     proper Content-Length framing.
  3. Reads the response from its stdout (parses a single `Content-Length: N\r\n\r\n` header followed by N bytes).
  4. Parses the response body as JSON, asserts:
     - `"jsonrpc": "2.0"`
     - `"id": 1` (matching the request)
     - `result.capabilities` is present and includes
       `textDocumentSync` (proves we registered the
       `TextDocumentSyncCapability` from `lsp_server::initialize`).
  5. Sends `exit` notification, then waits for the process to exit
     cleanly. (`shutdown` is a no-op; `exit` lets tower-lsp
     terminate gracefully.)
- Test must complete in under 5 seconds.
- Helper functions for framing live in the test file — no new
  dependencies. Use `serde_json` only if it's already in the dep
  tree; otherwise hand-roll a minimal JSON parser for the response
  shape we assert on.
- `cargo test --features lsp` passes; default `cargo test` is
  unchanged.
- `cargo clippy --features lsp -- -D warnings` clean.
- Commit message: `RES-090: LSP integration smoke test — initialize round trip`.

## Notes
- Use `std::process::Command` + `stdin().take()` + `stdout().take()`
  for I/O. Set `stdin/stdout` to `Stdio::piped()`.
- The LSP `initialize` request looks like:
  ```json
  {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}
  ```
- Don't pull in serde_json if not already there. Look for it under
  tower-lsp's dep tree — it's almost certainly transitively
  available (tower-lsp's deps include serde_json), so `extern crate`
  isn't needed; you can `use serde_json` directly under
  `--features lsp`.
- The full diagnostics flow (send didOpen, read publishDiagnostics
  notification) is a follow-up ticket once the framing helpers prove
  themselves.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
