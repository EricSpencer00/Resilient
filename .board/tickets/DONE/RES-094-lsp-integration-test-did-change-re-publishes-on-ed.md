---
id: RES-094
title: LSP integration test — didChange re-publishes diagnostics
state: DONE
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
- 2026-04-17 executor landed:
  - New `lsp_did_change_republishes_diagnostics` test in
    `tests/lsp_smoke.rs`. Reuses every helper from RES-090 +
    RES-093 (`frame`, `read_one_message`, `read_until`, `bin`).
  - Three-phase flow:
    1. didOpen `let x = 1;` → assert publishDiagnostics with
       `"diagnostics":[]` (empty / clean signal).
    2. didChange to `let bad: int = "hi";` (version 2) →
       assert non-empty diagnostics + typechecker wording.
    3. didChange revert to `let x = 1;` (version 3) → assert
       empty diagnostics again (proves stale errors clear).
  - Each phase uses a 5-second deadline.
  - Clean shutdown via `exit` notification within 3s.
- 2026-04-17 verification across three feature configs:
  - default: 217 unit + 1 golden + 11 smoke = 229 tests
  - `--features z3`: 225 + 1 + 12 = 238 tests
  - `--features lsp`: 221 + 1 + 11 + 3 lsp_smoke = 236 tests
  All three `cargo clippy -- -D warnings` clean.
- LSP track now has end-to-end coverage of all three editor-
  facing paths: handshake (RES-090), didOpen (RES-093),
  didChange (this ticket).
