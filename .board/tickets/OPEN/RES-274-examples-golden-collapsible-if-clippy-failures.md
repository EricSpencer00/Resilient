---
id: RES-274
title: "examples_golden.rs: collapsible_if clippy errors in expected_path and is_interactive"
state: OPEN
priority: P2
goalpost: G7
created: 2026-04-20
owner: executor
---

## Summary

The RES-272 implementation extended `resilient/tests/examples_golden.rs` with
two helper functions (`expected_path` and `is_interactive`) that contain nested
`if` blocks for handling `<dir>/main.res` multi-file examples. Clippy rejects
these with `clippy::collapsible_if` errors (6 total), blocking the CI gate.

Affected code in `tests/examples_golden.rs`:

```rust
// lines 68-76 in expected_path
if file_name == "main.res" {
    if let Some(parent) = example.parent() {
        if let Some(dir_name) = parent.file_name().and_then(|s| s.to_str()) {
            if let Some(grandparent) = parent.parent() {
                return grandparent.join(format!("{dir_name}.expected.txt"));
            }
        }
    }
}

// lines 90-98 in is_interactive (same pattern)
if file_name == "main.res" {
    if let Some(parent) = example.parent() {
        if let Some(dir_name) = parent.file_name().and_then(|s| s.to_str()) {
            if let Some(grandparent) = parent.parent() {
                return grandparent.join(format!("{dir_name}.interactive")).exists();
            }
        }
    }
}
```

Clippy reports 6 `collapsible_if` errors across these two blocks:
- `tests/examples_golden.rs:68`, `:69`, `:70` (in `expected_path`)
- `tests/examples_golden.rs:90`, `:91`, `:92` (in `is_interactive`)

## Acceptance criteria

- Both nested `if` blocks are collapsed using `&&`-chained conditions as
  clippy suggests. The canonical fix for each:

  ```rust
  if file_name == "main.res"
      && let Some(parent) = example.parent()
      && let Some(dir_name) = parent.file_name().and_then(|s| s.to_str())
      && let Some(grandparent) = parent.parent()
  {
      return grandparent.join(format!("{dir_name}.expected.txt"));
  }
  ```

  (Same pattern for `is_interactive`, returning `.exists()` instead.)

- No test logic is changed — only the `if`-nesting structure is collapsed.
  Existing test behaviour for `expected_path` and `is_interactive` is
  identical before and after.
- `cargo clippy --all-targets -- -D warnings` is clean (0 errors).
- `cargo test --manifest-path resilient/Cargo.toml` passes with 0 failures.
- Commit: `RES-274: collapse nested if blocks in examples_golden expected_path/is_interactive`.

## Notes

- These functions were added as part of RES-272 (golden coverage for
  `imports_demo/` multi-file example). The logic is correct; only the
  style needs updating to satisfy clippy.
- `let`-chaining in `if` conditions (`if let Some(x) = ... && let Some(y) = ...`)
  requires Rust edition 2024 or the `let_chains` feature. Confirm the
  crate's edition in `resilient/Cargo.toml` before using this form.
  If `let`-chaining is not available, use a single `if let (Some(a), Some(b), ...) = (...)` 
  tuple pattern or an early-return helper function instead.
- This is a **test file** change. Per CLAUDE.md test-protection policy,
  the PR must include a "Test changes" section with rationale.
  Rationale: the change is style-only (clippy compliance); no assertion
  or coverage is altered.

## Log

- 2026-04-20 created by analyzer (cargo clippy --all-targets -- -D warnings
  fails with 6 collapsible_if errors in tests/examples_golden.rs:68-98;
  introduced by RES-272 implementation)
