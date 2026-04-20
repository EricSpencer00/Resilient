---
id: RES-243
title: "imports_missing_file_errors_cleanly is flaky: shared fixed temp-file path causes parallel-test race"
state: DONE
priority: P2
goalpost: G11
created: 2026-04-20
owner: executor
claimed-by: Claude Sonnet 4.6
closed-by: 8140f3818e3d5241de80688cc7aaeb6ffb82c70c
---

## Summary

The smoke test `imports_missing_file_errors_cleanly`
(`resilient/tests/examples_smoke.rs`, ~line 570) fails intermittently when
`cargo test` runs tests in parallel. The test is **not** broken in isolation —
`cargo test imports_missing_file_errors_cleanly` consistently passes — but it
races with other tests when the full suite runs.

The root cause is a **fixed shared temp-file path**:

```rust
let tmp = std::env::temp_dir().join("res_073_missing_use.rs");
```

Because all threads in the same test run (and potentially leftover cleanup
from a prior run) share the path
`/var/folders/.../T/res_073_missing_use.rs`, two concurrent test
invocations can interleave:

1. Thread A calls `std::fs::File::create(&tmp)` — file now exists.
2. Thread B calls `std::fs::remove_file(&tmp)` (its own cleanup) — file deleted.
3. Thread A's spawned `resilient` binary tries to open the file → `ENOENT`.
4. Thread A's assertion fails: `stderr` contains `"Error reading file: ..."` instead
   of `"Import error: could not be resolved ..."`.

The fix is to use a unique per-invocation file name (e.g. via a random suffix
or the test's thread ID) so no two test instances share a path.

## Observed failure

```
---- imports_missing_file_errors_cleanly stdout ----
thread '...' panicked at tests/examples_smoke.rs:590:5:
expected import-error diagnostic, got:
seed=17869359124055498241
Error: Error reading file: No such file or directory (os error 2)
```

The `seed=` line confirms the `resilient` binary started successfully; the
error is that the temp source file was deleted by another thread between its
creation and the binary reading it.

## Relationship to existing tickets

IN_PROGRESS tickets `RES-239-imports-missing-file-test-regression.md` and
`RES-241-imports-missing-file-test-regression.md` both track this test
failure but hypothesise the root cause is in the import error-propagation
pipeline (`imports.rs` / `main.rs`). That hypothesis is **incorrect**:
manual testing of the import pipeline shows it works correctly and the test
passes consistently when run in isolation. The true cause is the race
documented here.

This ticket supersedes those hypotheses. Executors working on the imports
test failure should use this analysis instead.

## Acceptance criteria

- Change the temp-file path in `imports_missing_file_errors_cleanly` to
  use a unique name per invocation, e.g.:
  ```rust
  let tmp = std::env::temp_dir()
      .join(format!("res_073_missing_use_{}.rs", std::process::id()));
  ```
  Or use a crate like `tempfile` if one is already available (check
  `Cargo.toml` first — add no new deps if it isn't there).
- `cargo test` passes consistently across multiple runs including with
  `-- --test-threads=8` (or the default concurrency).
- `cargo clippy --all-targets -- -D warnings` remains clean.
- Do **not** modify any assertion logic in the test — only the temp-file
  path generation.
- Commit: `RES-243: fix flaky temp-file race in imports_missing_file_errors_cleanly`.

## Affected code

- `resilient/tests/examples_smoke.rs` — function `imports_missing_file_errors_cleanly`
  (the `tmp` path assignment, ~line 574).

## Notes

- `std::process::id()` is not unique if the same PID is reused across runs,
  but within a single `cargo test` invocation it is sufficient because tests
  within one binary run in the same process (different threads). A better
  option is `std::thread::current().id()` combined with a timestamp, or
  simply using `tempfile::NamedTempFile` if the crate is available.
- Since the test body runs in a single thread and the tmp path is
  created-at-start / deleted-at-end, using the thread ID is sufficient.
- The underlying import-error diagnostic code in `imports.rs` is correct
  and does NOT need to be changed.

## Log

- 2026-04-20 created by analyzer (root-cause analysis of the flaky test
  failure observed during `cargo test` parallel run)
