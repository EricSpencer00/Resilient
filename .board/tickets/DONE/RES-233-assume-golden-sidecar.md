---
id: RES-233
title: "Add `.expected.txt` golden sidecar for `assume()` examples"
state: DONE
Claimed-by: Claude Sonnet 4.6
priority: P3
goalpost: testing
created: 2026-04-20
owner: executor
---

## Summary

`assume()` was shipped in RES-133a with only unit tests inside
`#[cfg(test)]`. CLAUDE.md requires: _"New language features: add an
`.expected.txt` golden sidecar in `resilient/examples/`."_ No example
file or golden sidecar was added for the `assume` keyword.

## Context

The four unit tests in `src/main.rs` (lines 14934–14971) verify the
runtime behaviour, but the golden harness (`tests/examples_golden.rs`)
does not exercise assume at all. A user reading the `examples/`
directory has no demonstration of the feature.

## Acceptance criteria

- Add `resilient/examples/assume_demo.res` demonstrating:
  - `assume(expr)` passes silently when the expression is true.
  - `assume(expr, "msg")` with a custom message string is valid syntax.
  - At least one `assume` that feeds into a later computation so the
    output proves the code ran past it.
- Add `resilient/examples/assume_demo.expected.txt` capturing the exact
  stdout produced by `resilient examples/assume_demo.res`.
- `cargo test` remains fully green (including `golden_outputs_match`).
- `cargo test -- --ignored missing_expected_files_are_intentional`
  reports the same count as before (the new file has a sidecar).
- Commit message: `RES-233: golden sidecar example for assume()`.

## Notes

- Do **not** demonstrate `assume(false)` — that halts the program and
  the golden output would capture a failure message. Keep the demo
  affirmative.
- Do **not** modify existing tests or golden files.
- The example should be brief (< 15 lines) and self-contained.

## Log
- 2026-04-20 created by analyzer
