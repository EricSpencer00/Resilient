---
id: RES-095
title: VM runtime errors include source filename in driver output
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
RES-091 + RES-092 wired VM runtime errors with source line info, so
the user sees `vm: divide by zero (line 2)`. The typechecker uses
the richer `<file>:<line>:<col>:` form (RES-080) which is
editor-clickable — many editors auto-link such prefixes for
jump-to-source.

This ticket brings the VM driver output into the same shape: when
the binary catches a `VmError::AtLine`, it should print
`<filename>:<line>: <inner>` instead of the bare
`VM runtime error: <message> (line N)` form. Driver-only change;
the VM module's `Display` impl stays unchanged for callers that
want the line-suffix form.

## Acceptance criteria
- `execute_file` in `main.rs` (the `if use_vm { ... }` branch)
  inspects the returned `VmError`. If it's an `AtLine { line, kind }`
  variant, format the error as `<filename>:<line>: <kind>` (where
  `<kind>` is the inner Display); otherwise fall back to the
  current `VM runtime error: <message>` form.
- New smoke test in `tests/examples_smoke.rs` (default features —
  no need for `--features lsp` etc.):
  - Writes a temp file with a divide-by-zero on a known line.
  - Runs `--vm`.
  - Asserts stderr contains `<temp_path>:<line>:` prefix.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` pass
  on all three feature configs.
- Commit message: `RES-095: VM driver output prefixes errors with file:line:`.

## Notes
- The `VmError::kind()` helper (RES-091) returns the underlying
  variant — useful here too, since the formatter wants the
  innermost Display, not the wrapped one.
- Be careful: `kind()` returns `&VmError`. To Display its inner
  message without the `(line N)` suffix, just call its `Display`
  impl — `format!("{}", inner_kind)` produces `vm: divide by zero`
  with no line.
- Don't touch the `VmError::AtLine` Display itself — other
  callers (vm.rs unit tests, future LSP integration) want the
  line-suffix form.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
