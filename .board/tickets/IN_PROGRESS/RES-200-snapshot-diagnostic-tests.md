---
id: RES-200
title: Snapshot tests for diagnostic rendering (insta-style)
state: IN_PROGRESS
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

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
