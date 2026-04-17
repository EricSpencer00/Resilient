---
id: RES-080
title: G6 typechecker error messages use spans
state: DONE
priority: P1
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
RES-077 landed `Spanned<Node>` on every top-level statement. The
typechecker can now use that to attribute errors to a specific line
and column without waiting for the per-expression span work in
RES-078/079. Users currently see things like
`Type error: Expected int, got string` with no file location; this
ticket prepends `<file>:<line>:<col>: ` so the diagnostic surfaces
the exact statement that failed.

This is the user-visible deliverable that justifies the AST migration
work — it makes spans pay off immediately, before RES-078 / RES-079
go deep.

## Acceptance criteria
- New helper `TypeChecker::check_program_with_source(program: &Node, source_path: &str) -> Result<Type, String>` (keep the original `check_program` signature alongside as a thin shim that calls it with `source_path = "<unknown>"`).
- Inside `check_program`, the per-statement loop wraps each `check_node(&stmt.node)` call: if it returns `Err`, the error string is rewritten to `<source_path>:<line>:<col>: <original message>` using `stmt.span.start`.
- The driver (`execute_file` in `main.rs`) calls the new entry point with the path it was given. Existing callers of `check_program` (REPL, tests) keep the old signature.
- New unit test in `main.rs` `mod tests`: parse a multi-line program where the SECOND top-level statement violates a type rule (e.g. `let x = 1;\nfn f() { return \"oops\"; } /* if your type rule is enforced */ ...`). Pick whatever rule we already enforce and trip it. Assert that the resulting `Err(...)` string contains `:2:` (or wherever the offending statement starts) AND the file name. Use a synthetic `source_path = "test.rs"` for the test.
- An end-to-end smoke test in `tests/examples_smoke.rs`: write a temp file that triggers a known type error on a non-first line, run the binary with `--typecheck`, assert stderr contains `tempfile:LINE:COL:`.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all pass on default features and `--features z3`.
- Commit message: `RES-080: typechecker errors prefix file:line:col (G6 partial)`.

## Notes
- `check_program` is at `resilient/src/typechecker.rs:423`. The per-stmt loop is at the bottom of the `Node::Program` arm — call sites of `check_node(&stmt.node)?` are where the wrap-and-rewrite goes.
- Driver call site: `execute_file` in `main.rs` near `tc.check_program(&program)` — pass the `filename` parameter through.
- DO NOT touch every `Err(format!(...))` site in `check_node` itself. The wrapper at the top of the per-stmt loop is enough — sub-expression spans (RES-078/079) will let us refine attribution further later.
- For the test, pick a type rule that currently *does* fire. Look at `typed_let_parses_and_records_annotation` and surrounding tests for examples of what the typechecker enforces.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
- 2026-04-17 executor landed:
  - `TypeChecker::check_program_with_source(program, source_path)`:
    new entry point that wraps each per-stmt `check_node` call with a
    `map_err` that prepends `<source_path>:<line>:<col>: ` using the
    statement's `Spanned` start position from RES-077. Skips the
    prefix when `start.line == 0` (synthetic / unspanned).
  - `check_program` is now a thin shim that calls the new entry point
    with `source_path = "<unknown>"` — preserves all existing call
    sites (REPL, unit tests) without source-thread changes.
  - Driver `execute_file` switched its single call site to pass the
    real `filename` through.
- 2026-04-17 tests:
  - `typecheck_error_includes_file_line_col_prefix`: 2-line setup
    where the type error is on line 2 with annotation
    `let bad: int = "hi";` — asserts the resulting Err starts with
    `scratch.rs:2:`.
  - `check_program_legacy_shim_uses_unknown_source`: documents the
    backwards-compat behavior of the original entry point.
  - End-to-end `typecheck_error_prefixes_path_and_line` smoke test
    in `tests/examples_smoke.rs`: writes a temp file with the error
    on line 3, runs the binary with `--typecheck`, asserts stderr
    contains `:3:` and the temp path.
- 2026-04-17 manual verification: a real run prints
  `/tmp/r80.rs:3:5: let bad: int — value has type string` —
  navigable diagnostic.
- 2026-04-17 verification: 168 unit + 1 golden + 7 smoke = 176 tests
  default, 176 + 1 + 8 = 185 with `--features z3`. Clippy clean
  both ways.
