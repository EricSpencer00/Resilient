---
id: RES-093
title: LSP integration test — didOpen publishes diagnostics
state: OPEN
priority: P2
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
RES-090 proved the LSP server's `initialize` handshake works.
This ticket completes the LSP integration story: send `didOpen`
with a known-buggy program, read the resulting
`publishDiagnostics` notification, and assert at least one
diagnostic with a reasonable `Range`.

Reuses the framing helpers landed in `tests/lsp_smoke.rs` —
no new infrastructure. Adds a second test in the same file.

## Acceptance criteria
- New test `lsp_did_open_publishes_diagnostics` in
  `resilient/tests/lsp_smoke.rs` (gated on `--features lsp`).
- Test flow:
  1. Spawn `resilient --lsp` with piped stdio.
  2. Send `initialize` request, read response (matches the
     existing test).
  3. Send `initialized` notification.
  4. Send `textDocument/didOpen` notification with a 3-line
     program where the third line is a known type error
     (e.g. `let bad: int = "hi";`).
  5. Read framed messages from stdout until one contains
     `"method":"textDocument/publishDiagnostics"` (skip
     anything else like log messages). Cap with a 5-second
     deadline.
  6. Substring-assert the notification body contains:
     - `"diagnostics"` (the array key)
     - the substring of the original error (e.g. `"let bad: int"`
       or `"string"`)
     - a `"line":2` (0-indexed → source line 3 = LSP line 2)
  7. Send `exit` notification, wait for clean exit.
- Helpers from the existing test (`frame`, `read_one_message`,
  `bin`) reused. Add a small `read_message_matching(predicate)`
  loop helper if convenient.
- `cargo test --features lsp` passes; default `cargo test`
  unchanged.
- `cargo clippy --features lsp -- -D warnings` clean.
- Commit message: `RES-093: LSP integration test — didOpen publishes diagnostics`.

## Notes
- The LSP server publishes diagnostics asynchronously after
  `did_open` returns. The test's read loop should NOT assume
  the very next message is publishDiagnostics — log messages
  from `initialized()` etc. may arrive first.
- A `textDocument/didOpen` looks like:
  ```json
  {"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
    "textDocument":{
      "uri":"file:///tmp/scratch.rs",
      "languageId":"resilient",
      "version":1,
      "text":"let a = 1;\nlet b = 2;\nlet bad: int = \"hi\";"
    }
  }}
  ```
- Use a real-looking `file://` URI so the typechecker's path
  prefix in error messages is a valid URL string.
- 0-indexed LSP line: source line 3 → `"line":2`. The test
  asserts on the substring `"line":2` (allowing whitespace
  inside the JSON). If the JSON has `"line": 2` with a space,
  use `"line": 2"` or split on regex.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
