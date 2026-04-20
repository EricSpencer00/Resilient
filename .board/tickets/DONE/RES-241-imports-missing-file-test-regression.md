---
id: RES-241
title: "Test regression: imports_missing_file_errors_cleanly expects clean diagnostic but gets OS error"
state: SUPERSEDED
priority: P2
goalpost: G11
created: 2026-04-20
owner: executor
Claimed-by: Claude
superseded-by: RES-243
---

## Summary

The test `imports_missing_file_errors_cleanly` in
`resilient/tests/examples_smoke.rs` is failing. The test creates a Resilient
source file with a `use "definitely-not-here.rs";` statement and expects the
compiler to emit a clean import-error diagnostic like `"could not be resolved"`.
Instead, it is receiving a raw OS-level error:

```
Error reading file: No such file or directory (os error 2)
```

This is a regression — RES-073 shipped with all tests passing, including this
one. The proper error handling exists in `resilient/src/imports.rs` (the
`resolve_use_path` function returns clean error messages), so the issue is
likely that:

1. The import resolution flow has changed, or
2. A recent change allows OS errors to escape before imports are processed, or
3. Error handling somewhere in the import pipeline has been weakened.

## Test details

Location: `resilient/tests/examples_smoke.rs`, function
`imports_missing_file_errors_cleanly` (around line 570).

Expected stderr to contain: `"Import error"` or `"could not be resolved"`

Actual stderr:
```
seed=<random>
Error reading file: No such file or directory (os error 2)
```

## Reproduction

```bash
cd resilient
cargo test imports_missing_file_errors_cleanly 2>&1 | grep -A 20 "imports_missing_file_errors_cleanly"
```

## Acceptance criteria

- Investigate the import resolution flow and error handling.
- Identify where the raw OS error "Error reading file: ..." is being
  generated and why it is not being converted to a clean import diagnostic.
- Fix the issue so the test passes, emitting a diagnostic containing
  "could not be resolved" (from `imports.rs:resolve_use_path`).
- `cargo test` must pass fully (699 → 700 tests).
- `cargo clippy --all-targets -- -D warnings` must remain clean.
- Commit message: `RES-239: fix import-error diagnostic regression`.

## Affected code

- `resilient/src/main.rs` — likely where OS errors are handled
- `resilient/src/imports.rs` — where clean error messages are defined
- Investigation needed to trace the exact flow.

## Notes

- This is a blocking test failure that must be fixed before merging.
- The imports.rs module has the correct error handling; the issue is
  likely in how it is called or how errors are propagated from main.rs.
- RES-073 is the parent ticket that originally shipped import support and
  this test.

## Dependencies

- Blocks any PR merge due to test failure.

## Log
- 2026-04-20 created by analyzer (found during `cargo test` run)
- 2026-04-20 superseded by RES-243 — the root-cause hypothesis in this
  ticket (broken import-error propagation pipeline) is incorrect. The test
  passes consistently when run in isolation. The true cause is a parallel-test
  race on a fixed shared temp-file path; see RES-243 for the correct
  analysis and fix. No code changes needed in imports.rs or main.rs.
