---
id: RES-271
title: "examples_golden.rs: stale *.rs file extension in module doc comment"
state: OPEN
priority: P4
goalpost: G17
created: 2026-04-20
owner: executor
---

## Summary

`resilient/tests/examples_golden.rs` line 3 contains a stale reference to
the old `.rs` file extension for Resilient source files:

```rust
//! For every `examples/<name>.rs` that has a sibling
```

After the `.rs → .res` rename, this should read:

```rust
//! For every `examples/<name>.res` that has a sibling
```

The production logic (`list_examples`) already correctly filters for `*.res`
files (line 33). Only the doc comment is stale.

RES-260 tracked similar stale `*.rs` comments in `lsp_server.rs`. This ticket
covers the same class of defect in `examples_golden.rs`.

## Acceptance criteria

- Line 3 of `resilient/tests/examples_golden.rs` is updated from
  `examples/<name>.rs` to `examples/<name>.res`.
- No production logic is changed — comment-only edit.
- `cargo test --test examples_golden` passes with 0 failures.
- Commit: `RES-271: fix stale *.rs comment in examples_golden.rs`.

## Notes

- This is a one-line comment fix. No test changes required.
- Do NOT change the `*.rs` references that refer to actual Rust source files
  (e.g. `lsp_smoke.rs`, `main.rs`).

## Log

- 2026-04-20 created by analyzer (examples_golden.rs line 3 says `<name>.rs`
  after the .rs→.res rename; same class as RES-260)
