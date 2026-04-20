---
id: RES-225
title: `resilient check` subcommand — type-check without running
state: OPEN
priority: P3
goalpost: G14
created: 2026-04-20
owner: executor
claimed-by: Claude
---

## Summary
Add `resilient check <file>` that runs the parser, type-checker, and verifier but does not execute the program. Analogous to `cargo check`. Useful in editor integrations and CI pipelines.

## Acceptance criteria
- `resilient check <file>` exits 0 if the file parses and type-checks cleanly.
- Any parse/type/verifier error prints to stderr with `file:line:col: error: ...` format and exits non-zero.
- `--features z3` additionally runs the Z3 verifier pass.
- `--quiet` / `-q` flag suppresses all output except exit code.
- `resilient --help` lists `check` as a subcommand.
- Integration test: `resilient check hello.rs` exits 0; `resilient check` on a file with a known type error exits 1.
- LSP server updated to call `resilient check` for on-save diagnostics once this lands.
- Commit message: `RES-225: \`resilient check\` subcommand — type-check without running`.

## Notes
- Reuse the existing `compile()` path in `compiler.rs` up to (but not including) the `run()` call.
- Subcommand parsed in `main.rs` via existing `clap` argument parser.

## Log
- 2026-04-20 created by manager
</content>
</invoke>