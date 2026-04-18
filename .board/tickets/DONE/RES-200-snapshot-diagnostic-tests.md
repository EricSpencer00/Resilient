---
id: RES-200
title: Snapshot tests for diagnostic rendering (insta-style)
state: DONE
priority: P3
goalpost: testing
created: 2026-04-17
owner: executor
---

## Summary
Diagnostic messages are load-bearing UX. Tests that assert on
exact message strings break every time we reword; tests that
only assert on substrings let silent regressions slide. Snapshot
tests strike the right balance: a reviewer approves the new
rendering, and unintended changes surface as diffs.

## Acceptance criteria
- Dev dep: `insta = "1"`.
- New `tests/diagnostics_snapshots.rs` with ~15 canary programs
  each triggering a specific diagnostic class (type error, missing
  semi, unknown ident, parser panic recovery, verifier failure,
  lint hit).
- Each test renders the program through the full driver and
  snapshots the stderr+stdout output.
- Instructions in CONTRIBUTING.md (or top-of-tests comment): to
  update snapshots, `cargo insta review`.
- Commit the initial `*.snap` files under `tests/snapshots/`.
- Commit message: `RES-200: snapshot tests for diagnostic output`.

## Notes
- Insta surfaces diffs inline at review time; CI should fail loudly
  rather than auto-accept. `insta::assert_snapshot!` is the right
  API (not `assert_display_snapshot!`, which does Display
  formatting and can hide surprises).
- Keep programs tiny — snapshots should be < 20 lines each.

## Resolution

### Files changed
- `resilient/Cargo.toml` — new `[dev-dependencies]` section adding
  `insta = { version = "1", features = ["filters"] }`. The
  `filters` feature exposes `Settings::add_filter` which is used
  to pin run-to-run-dynamic fragments (temp-file paths, `seed=<N>`
  stderr line) to stable placeholders.
- `resilient/Cargo.lock` — pick up `insta` + its transitive deps
  (`similar`, `console`, `regex`, `tempfile`, …) into the lockfile.
- `resilient/tests/diagnostics_snapshots.rs` — new test file, 16
  canary programs (5 parser-side, 5 typechecker-side, 6 runtime-
  side). Each writes its source to a scratch `<tmp>.rs`, runs the
  real `resilient` binary, captures `stdout + stderr`, normalizes,
  and calls `insta::assert_snapshot!`.
- `resilient/tests/snapshots/*.snap` — 16 committed snapshot
  files.

### Design choices
- **Merged stdout + stderr** into one capture buffer so the
  snapshot shows exactly what a user sees in a terminal.
- **UTF-8-aware ANSI stripper** — the driver emits colored output
  with em-dashes inside. The first naive byte-loop corrupted the
  em-dashes; rewritten to iterate chars.
- **Path normalization in two layers**:
  - Literal `.replace()` on the scratch-file path (most tests
    write to `/tmp/res_snap_<tag>_<pid>_<n>.rs`).
  - Regex `add_filter` as a safety net for any path that escapes
    the first pass (e.g. `/var/folders/...` on macOS).
- **`--seed 0` on every invocation** so the CLI's default seed-
  echo (`seed=<N>` on stderr when no `--seed` is passed) never
  reaches the snapshot. A filter as backup in case a canary
  omits the flag.
- **Per-test `set_snapshot_suffix`** so the committed files have
  descriptive names (`diagnostics_snapshots__<test>@<suffix>.snap`)
  rather than numeric `@1`, `@2`, etc.

### Coverage
The 16 canaries exercise:
- **Parser panic recovery** — missing `=` in `let`, unexpected token
  after `fn`, missing `=>` in `match`, missing `(` after `assert`,
  missing identifier after `let`.
- **Typechecker** — `int`-annotated binding with string value,
  undefined variable, arity mismatch at call, non-bool `if`
  condition, `+` on array.
- **Runtime** — division by zero, array index OOB, `assert(false)`,
  `unwrap(Err(…))`, contract (`requires`) violation, unknown
  identifier.

### Verification
- `cargo test --test diagnostics_snapshots` → 16 passed
- `cargo test --locked` → 478 + 16 = 494 tests pass (the extras
  on the core + other tests feature-gate)
- `cargo test --locked --features lsp` → 495 + 16 = 511 pass
- `cargo clippy --locked --features lsp,z3,logos-lexer --tests --
  -D warnings` → clean
- Ran twice back-to-back with different process IDs to confirm
  snapshots are stable across runs.

### Reviewer instructions
Comment at the top of `tests/diagnostics_snapshots.rs` explains
the review flow (`cargo insta review` to promote a `.snap.new`).
Kept inline per ticket's "CONTRIBUTING.md or top-of-tests comment"
option — no repo CONTRIBUTING.md exists yet.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (16 snapshot canaries covering
  parser / typechecker / runtime diagnostic classes)
