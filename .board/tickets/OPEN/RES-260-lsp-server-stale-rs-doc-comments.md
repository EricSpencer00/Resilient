---
id: RES-260
title: "lsp_server.rs: stale *.rs doc comments and module header after .rs→.res rename"
state: OPEN
priority: P3
goalpost: G17
created: 2026-04-20
owner: executor
---

## Summary

The `.rs → .res` file extension rename updated the runtime `walk_resilient_files`
implementation (correctly matches `*.res` today) and RES-246 tracks the
two test functions that still create `*.rs` scratch files. However, a
parallel set of stale references remains in **doc comments and inline
comments** across `lsp_server.rs` that still say `*.rs` where they should
say `*.res`.

Additionally the module-level doc comment at line 8 still reads:

```rust
//! Nothing else yet — no hover, no completion, no go-to-definition.
```

This is now incorrect: hover landed in RES-181a, go-to-definition landed
in RES-182, and find-references landed in RES-183.

## Stale locations (as of commit 6b55fe4)

| Line | Stale text | Correct text |
|---|---|---|
| 8 | `no hover, no completion, no go-to-definition` | update to reflect shipped features |
| 59 | `` `*.rs` file's top-level symbols `` | `` `*.res` file's top-level symbols `` |
| 790 | `index_file handles one *.rs at a time` | `index_file handles one *.res at a time` |
| 815 | `RES-186: recursive *.rs walker` | `RES-186: recursive *.res walker` |
| 1245 | `workspace-symbol search across all .rs` | `workspace-symbol search across all .res` |
| 1383 | `walks the workspace root for *.rs` | `walks the workspace root for *.res` |

## Acceptance criteria

- Every doc comment and inline comment in `resilient/src/lsp_server.rs`
  that refers to `*.rs` (as a Resilient source file extension) is updated
  to `*.res`.
- The module-level doc comment at line 8 is updated to list the
  capabilities that have since landed (hover, go-to-definition,
  find-references, completion, semantic tokens).
- No production logic is changed — comment-only edits.
- `cargo clippy --all-targets --features lsp -- -D warnings` clean.
- `cargo test --features lsp` passes with 0 failures.
- Commit: `RES-260: fix stale *.rs doc comments in lsp_server.rs`.

## Notes

- This ticket is comment-only. Do NOT change any `*.rs` references that
  refer to actual Rust source files (e.g. `lsp_smoke.rs`, `main.rs`,
  `"scratch.rs"` in test error strings that simulate Rust-file URIs for
  diagnostic-parsing tests).
- The two test-code `*.rs` regressions are tracked separately in RES-246
  and require maintainer approval per CLAUDE.md test-protection policy.
- `pkg_init.rs` line 180 also has a `*.rs` comment — include it if it
  refers to Resilient source files (check the context).

## Log

- 2026-04-20 created by analyzer (stale *.rs doc comments found in
  lsp_server.rs after .rs→.res rename; module header also stale)
