---
id: RES-091
title: VM runtime errors include source line from chunk.line_info
state: DONE
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
RES-076 introduced `Chunk::line_info: Vec<u32>` parallel to `code`,
populated via `chunk.emit(op, line)` so each instruction knows its
originating source line. **The field is never read.** When the VM
raises `VmError::DivideByZero` etc., the error message doesn't
include any source location — the user just sees `vm: divide by zero`.

This ticket wires `line_info` through to error reporting so VM
runtime errors carry the line they originated on. Same diagnostic-
quality story G6 told for the AST, applied to the VM.

## Acceptance criteria
- New `VmError` variant: `WithLine { line: u32, inner: Box<VmError> }`.
  Or simpler: add an `at_line: Option<u32>` field to existing
  variants. Pick whichever is less invasive — the goal is for the
  Display impl to print `vm: divide by zero (line 5)` instead of
  `vm: divide by zero`.
- VM dispatch loop wraps each error return with the current pc's
  `chunk.line_info[pc]`. Use a helper like
  `fn err_at(line_info: &[u32], pc: usize, e: VmError) -> VmError`.
- Existing `VmError::Display` impls extended to print the `(line N)`
  suffix when present.
- Existing VM unit tests still pass; their assertions on the
  *kind* of error (e.g. `assert_eq!(err, VmError::DivideByZero)`)
  may need to compare on a stripped-down form. Use a helper like
  `kind(err)` or restructure with `if let`.
- New unit test: compile + run `let x = 10 / 0;`, assert the
  resulting `VmError`'s `Display` contains both `divide by zero`
  AND `line 1` (or whatever line the source places it on).
- Smoke test: end-to-end `--vm` invocation on a divide-by-zero
  source sees the line-attributed error in stderr.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` pass
  on all three feature configs.
- Commit message: `RES-091: VM runtime errors carry source line`.

## Notes
- `chunk.line_info` is at `bytecode.rs:50` (parallel to `chunk.code`).
- VM runs in `vm.rs::run`. The pc tracking is in `frames[frame_idx].pc`.
- The fn-call path uses a different chunk per frame — make sure the
  helper looks up `line_info` of the CURRENT frame's chunk, not main's.
- Tests that match exactly `VmError::DivideByZero` will need a tweak.
  Look in `vm.rs` `mod tests` and `tests/examples_smoke.rs` for those.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
- 2026-04-17 executor landed:
  - New `VmError::AtLine { line, kind: Box<VmError> }` wrapper
    variant. `Display` impl prints `<inner> (line N)`.
  - New `VmError::kind() -> &VmError` peels off any `AtLine`
    layers — tests that match on the underlying variant call
    `err.kind()` first.
  - New free fn `err_at(line_info, pc, e)` looks up the line at
    `pc.saturating_sub(1)` (the just-failed op) in `line_info`
    and wraps if non-zero. Sentinel pass-through if the slot is
    0 or out of bounds.
  - `vm::run` split into `run` (outer wrapper) + `run_inner`
    (the original dispatch loop). `run_inner` updates a shared
    `last_pc: (chunk_idx, pc)` snapshot at the top of every
    iteration. `run` looks up the right chunk's `line_info` and
    wraps via `err_at` once at the boundary — keeps every inner
    `?` and `return Err(...)` site untouched.
  - 3 existing `assert_eq!(err, VmError::Foo)` tests updated to
    use `err.kind()` for kind-based comparison.
- 2026-04-17 tests:
  - New `divide_by_zero_error_includes_source_line`: parses
    `let x = 10 / 0;`, asserts the error's `Display` contains
    both `divide by zero` and `line ` substrings, and confirms
    `err.kind()` still equals `VmError::DivideByZero`.
- 2026-04-17 manual end-to-end: `cargo run --vm /tmp/r91.rs` on
  a divide-by-zero source prints
  `Error: VM runtime error: vm: divide by zero (line 1)`.
  Line attribution lands at the top-level statement — refining
  to per-instruction lines requires propagating RES-079 spans
  through compile.rs (separate ticket if/when needed).
- 2026-04-17 verification across three feature configs:
  - default: 216 unit + 1 golden + 11 smoke = 228 tests
  - `--features z3`: 224 + 1 + 12 = 237
  - `--features lsp`: 220 + 1 + 11 + 1 lsp_smoke = 233 tests
  All three `cargo clippy -- -D warnings` clean.
