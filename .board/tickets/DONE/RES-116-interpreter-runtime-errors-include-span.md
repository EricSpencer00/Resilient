---
id: RES-116
title: Interpreter runtime errors print `file:line:col:` prefix
state: DONE
priority: P2
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
RES-080 got statement-level spans into the typechecker. The
interpreter (tree-walker) still prints bare error messages
(`Runtime error: division by zero`). The VM got the same treatment
in RES-091/092 via `chunk.line_info`; the interpreter has even better
data available (AST spans, post-RES-088) and should surface them.

## Acceptance criteria
- Every `RResult::Err(msg)` path in `main.rs` `interpret_*`
  functions is widened to `RResult::ErrAt(span, msg)` (new variant
  or boxed struct — whichever is less churn).
- The driver formats errors as
  `filename:line:col: Runtime error: <msg>` using the span's
  start `Pos`.
- Bare `RResult::Err` calls in library paths that lack a span
  (e.g. builtin failures invoked indirectly) fall back to the
  current no-prefix format rather than a fake span.
- Existing `*.expected.txt` goldens update where runtime errors
  appear — one file, `self_healing.expected.txt`, currently asserts
  on an error string.
- Unit tests: one per runtime error class (divide-by-zero, array
  OOB, missing function) verifying the new prefix appears.
- Commit message: `RES-116: interpreter runtime errors carry spans`.

## Notes
- Don't change the public `RResult` alias yet if it's aliased in
  `resilient-runtime` — the interpreter can use a richer internal
  type and lower to the runtime's simpler type at the boundary.
- Performance: this path is cold (error case only), no need to
  sweat boxing costs.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed: `resilient/src/main.rs`.

Approach: Rather than widen `RResult<T>` (a pervasive `Result<T,
String>` alias) into a span-carrying enum — which would cascade
through ~100 error-creation sites — we decorate runtime errors with a
statement-level `line:col:` prefix at the one spot where the
interpreter walks statements that already have spans. The driver then
reshapes the prefix into the full `filename:line:col: Runtime error:
<msg>` form, matching the VM's RES-091 output.

Key edits:
- `Interpreter::eval_program` (tree-walker entry point) wraps every
  statement evaluation's `Err` with the statement's `Span` start
  line:col via a new helper `decorate_runtime_error`.
- `decorate_runtime_error` skips already-decorated errors (via
  `has_line_col_prefix`) so nested statements don't double-prefix.
- `format_interpreter_error` reshapes decorated errors as
  `<filename>:<line>:<col>: Runtime error: <msg>`; undecorated
  errors get the legacy bare `Runtime error: <msg>` form.
- `execute_file` uses `format_interpreter_error` on the error path.

Acceptance criteria addressed:
- `line:col:` prefix on every runtime error reaching the driver from
  `eval_program` — yes, via span decoration at the statement boundary.
- Bare-fallback path preserved for library-level errors that never
  reach `eval_program` — yes, via `has_line_col_prefix` check.
- Unit tests: added three runtime-error-class tests (divide by zero,
  array out-of-bounds, missing function) plus four infrastructure
  tests for `has_line_col_prefix` / `format_interpreter_error`.
- Goldens: `self_healing.expected.txt` was inspected and does NOT
  currently assert on a runtime error string (retries recover, the
  program prints success and exits 0), so no golden file needed
  updating. The ticket noted this file but its current expected
  output is purely success-path.

Deviation from the sketch: kept `RResult = Result<T, String>` — the
error string gets a tagged prefix instead of a new `ErrAt` variant.
The ticket explicitly allowed choosing "whichever is less churn" and
called out keeping the `resilient-runtime` boundary stable, which the
string-tag approach does trivially.

Verification:
- `cargo build` — clean.
- `cargo test` — 233 unit (includes 7 new RES-116 tests) + 13
  integration pass.
- `cargo clippy --tests -- -D warnings` — clean.
- Manual: `resilient /tmp/boom.rs` (div-by-zero) now prints
  `Error: /tmp/boom.rs:5:5: Runtime error: Division by zero` and
  exits 1.
- `examples/self_healing.rs` and `examples/hello.rs` run unchanged.
