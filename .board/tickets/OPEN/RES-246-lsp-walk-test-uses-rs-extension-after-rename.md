---
id: RES-246
title: "LSP: two walk_resilient_files tests fail after .rs→.res rename"
state: OPEN
priority: P2
goalpost: G17
created: 2026-04-20
owner: executor
---

## Summary

Commit 94595a5 ("MGR: rename .rs → .res, wire VS Code extension …")
updated `walk_resilient_files` in `lsp_server.rs` to match `*.res`
files instead of `*.rs`, but two unit tests inside `lsp_server::tests`
were not updated. They still create scratch files with `.rs` names, so
`walk_resilient_files` skips them and the assertions fail.

Failing tests (reproduce with `cargo test --features lsp`):

| Test | Failure |
|---|---|
| `lsp_server::tests::walk_resilient_files_finds_rs_files_recursively` | `assertion failed: names.contains(&"a.rs".to_string())` |
| `lsp_server::tests::workspace_index_spans_multiple_files` | `assertion 'left == right' failed: left: 0 right: 2` |

## Root cause

`walk_resilient_files` (line 643) now checks:
```rust
path.extension().and_then(|s| s.to_str()) == Some("res")
```

But `walk_resilient_files_finds_rs_files_recursively` (line 1621) creates
`a.rs`, `b.rs`, `c.rs`, `d.rs`, and `workspace_index_spans_multiple_files`
(line 1964) creates `mod_a.rs`, `mod_b.rs`. Neither set is found by the
updated walker.

## Acceptance criteria

- Update the two failing tests so they create `*.res` files (not `*.rs`):
  - `walk_resilient_files_finds_rs_files_recursively`:  
    rename `a.rs` → `a.res`, `b.rs` → `b.res`, `c.rs` → `c.res`,
    `d.rs` → `d.res` and update the `names.contains(...)` assertions.
  - `workspace_index_spans_multiple_files`:  
    rename `mod_a.rs` → `mod_a.res`, `mod_b.rs` → `mod_b.res`,
    and update the path suffix assertions.
- `cargo test --features lsp` passes with 0 failures.
- `cargo clippy --all-targets --features lsp -- -D warnings` remains clean.
- Commit message: `RES-246: fix LSP walk tests to use .res extension after rename`.

## Notes

- CLAUDE.md requires maintainer approval for changes to existing tests.
  These are test-only changes fixing a test regression (not weakening
  assertions), so they qualify as bug fixes. Call out in the PR description
  under a **"Test changes"** section with rationale.
- Do NOT change the production `walk_resilient_files` implementation — it
  is correct. Only the test scaffolding needs updating.
- The `index_file_parses_and_extracts_top_level_symbols` test at line 1651
  uses `prog.rs` but calls `index_file` directly (not through
  `walk_resilient_files`), so it is NOT affected and should NOT be changed.

## Affected file

- `resilient/src/lsp_server.rs` — two test functions (lines ~1619–1645
  and ~1960–1995)

## Log
- 2026-04-20 created by analyzer (walk tests fail with --features lsp after .rs→.res rename)
