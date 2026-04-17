---
id: RES-091
title: VM runtime errors include source line from chunk.line_info
state: OPEN
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
