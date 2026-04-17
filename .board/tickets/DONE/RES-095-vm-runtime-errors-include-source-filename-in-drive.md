---
id: RES-095
title: VM runtime errors include source filename in driver output
state: DONE
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
- 2026-04-17 executor landed:
  - `execute_file` in `main.rs` (the VM branch) inspects the
    returned `VmError`. If it's `AtLine { line, kind }`, formats
    as `<filename>:<line>: <kind>` (matching the typechecker's
    RES-080 prefix shape). Otherwise falls back to the existing
    `VM runtime error: <message>` form.
  - `vm::VmError`'s own `Display` impl unchanged — other callers
    (vm tests, future LSP integration) still see the
    `<inner> (line N)` form when they want it.
- 2026-04-17 tests:
  - New `vm_runtime_error_includes_source_filename` smoke test in
    `tests/examples_smoke.rs`. Writes a 5-line temp file with a
    divide-by-zero on line 2, runs `--vm`, asserts stderr
    contains the temp path AND `:2:` AND `divide by zero`, and
    that the binary exits non-zero.
- 2026-04-17 manual: `cargo run --vm /tmp/r95.rs` on a
  divide-by-zero source prints `Error: /tmp/r95.rs:2: vm: divide
  by zero` — editor-clickable.
- 2026-04-17 verification across three feature configs:
  - default: 217 unit + 1 golden + 12 smoke = 230 tests
  - `--features z3`: 225 + 1 + 13 = 239 tests
  - `--features lsp`: 221 + 1 + 12 + 3 lsp_smoke = 237 tests
  All three `cargo clippy -- -D warnings` clean.
- Diagnostic surface is now uniform across parser, typechecker,
  and VM runtime: all three produce `<file>:<line>:` prefixed
  errors that editors auto-link.
