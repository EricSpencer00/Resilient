---
id: RES-272
title: "imports_demo/ subdirectory is never run by the golden test harness"
state: DONE
priority: P3
goalpost: G7
created: 2026-04-20
owner: executor
---

## Summary

`resilient/examples/imports_demo/` contains a two-file multi-module example
(`main.res` + `helpers.res`) but is never exercised by the golden test
harness in `resilient/tests/examples_golden.rs`.

`list_examples()` in `examples_golden.rs` (line 29-36) only reads the
top-level `examples/` directory and filters for `*.res` files:

```rust
fn list_examples() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(examples_dir())
        .expect("reading examples dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("res"))
        .collect();
    out.sort();
    out
}
```

`imports_demo/` is a directory, so `path.extension()` returns `None` and it
is silently excluded. Because the harness never runs `imports_demo/main.res`,
regressions in the imports feature would not be caught by `golden_outputs_match`.

RES-073 added the imports feature and the `imports_demo/` directory but
relied on a separate unit test (`imports_demo_resolves_use_clause`). That
unit test exercises the imports resolver in isolation, but not the full
end-to-end CLI execution that a golden test would catch.

## Acceptance criteria

- An `imports_demo.expected.txt` golden sidecar is added at
  `resilient/examples/imports_demo.expected.txt` (sibling of the directory,
  not inside it), OR the golden harness is extended to descend into
  subdirectories and look for `main.res` + `<dir>.expected.txt` pairs.
  Either approach is acceptable; the simpler one is a sidecar at the
  top level.
- The `list_examples()` function (or a companion function) is extended to
  include the `imports_demo/main.res` entry point. The simplest extension:
  also collect `examples/<dir>/main.res` paths where the corresponding
  `examples/<dir>.expected.txt` (sibling of the directory, at the top level)
  exists.
- `golden_outputs_match` runs `imports_demo/main.res` and asserts its
  output matches the new golden file.
- All existing golden tests continue to pass.
- `cargo test --test examples_golden` passes with 0 failures.
- Commit: `RES-272: add golden coverage for imports_demo multi-file example`.

## Notes

- Run `resilient examples/imports_demo/main.res` from the crate root to see
  the expected output and capture it into the golden file.
- The multi-file case requires the binary to be invoked from the crate root
  (or with an appropriate `--cwd` flag) so the `use` clause can resolve
  `helpers.res` relative to `main.res`. Check how the existing `run()`
  helper in `examples_golden.rs` sets `current_dir`.
- Do NOT modify existing golden `.expected.txt` files or existing tests.

## Log

- 2026-04-20 created by analyzer (imports_demo/ subdirectory is silently
  excluded by list_examples() in examples_golden.rs; no golden coverage
  for the multi-file imports feature)
closed-by: shipped in commit 0552548 (main)
