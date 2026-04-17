---
id: RES-094
title: LSP integration test — didChange re-publishes diagnostics
state: OPEN
priority: P2
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
RES-090 covered the initialize handshake; RES-093 covered the
didOpen path. This ticket completes the trio with a `didChange`
integration test — the path real editors hit on every keystroke.

The flow:
1. Open a CLEAN program (no errors) → empty diagnostics arrive.
2. Send `didChange` with a buggy version → non-empty diagnostics
   arrive with the new error's range.
3. Send another `didChange` reverting to clean → empty diagnostics
   again.

This proves the LSP server (a) reacts to edits and (b) clears
stale diagnostics when the buggy code is fixed — both behaviors
editors care about.

## Acceptance criteria
- New test `lsp_did_change_republishes_diagnostics` in
  `resilient/tests/lsp_smoke.rs` (gated `--features lsp`).
- Test flow:
  1. Spawn `resilient --lsp`.
  2. initialize → drain response.
  3. initialized notification.
  4. `didOpen` with a 1-line clean program: `let x = 1;`.
  5. Read until publishDiagnostics arrives. Assert the
     diagnostics array is EMPTY (`"diagnostics":[]`).
  6. `didChange` to `let bad: int = "hi";` — same URI, version 2,
     full content replace (we registered `TextDocumentSyncKind::FULL`).
  7. Read until next publishDiagnostics. Assert it's NOT empty
     and contains the typechecker wording.
  8. `didChange` again, reverting to `let x = 1;` (version 3).
  9. Read until next publishDiagnostics. Assert empty again.
  10. exit + clean shutdown.
- Reuse `frame()`, `read_one_message()`, `read_until()`, `bin()`
  from existing tests. No new helpers required.
- `cargo test --features lsp` passes; default `cargo test`
  unchanged.
- `cargo clippy --features lsp -- -D warnings` clean.
- Commit message: `RES-094: LSP integration test — didChange re-publishes diagnostics`.

## Notes
- A `textDocument/didChange` notification with FULL sync looks like:
  ```json
  {"jsonrpc":"2.0","method":"textDocument/didChange","params":{
    "textDocument":{"uri":"file:///tmp/lsp_change.rs","version":2},
    "contentChanges":[{"text":"let bad: int = \"hi\";"}]
  }}
  ```
- LSP's `publishDiagnostics` is a notification (no `id`). The
  predicate in `read_until` filters on the method name to skip
  unrelated messages.
- An EMPTY diagnostics array publication is the LSP server's way
  of saying "all clear" — assert on the substring `"diagnostics":[]`.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
