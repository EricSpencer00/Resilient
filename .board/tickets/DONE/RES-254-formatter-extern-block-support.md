---
id: RES-254
title: "`resilient fmt` silently drops `extern` blocks"
state: DONE
priority: P2
goalpost: tooling
created: 2026-04-20
owner: executor
Claimed-by: Claude Sonnet 4.6
---

## Summary

The `resilient fmt` formatter in `resilient/src/formatter.rs` contains a
comment-and-skip for `Node::Extern`:

```rust
// FFI v1: extern blocks not yet formatted (Tasks 4-8).
Node::Extern { .. } => {}
```

Any Resilient source file that contains an `extern` block will have those
blocks silently erased when formatted with `resilient fmt --in-place`. This
is a silent data-loss bug for FFI users.

## Evidence

- `resilient/src/formatter.rs` line 357-358: `Node::Extern { .. } => {}` in
  `fmt_stmt`.
- `resilient/src/formatter.rs` line 656: `Node::Extern { .. }` in the
  expression fallback arm (also emits nothing).
- `resilient/examples/ffi_libm.res` is the canonical FFI example that would
  be corrupted by `fmt --in-place`.

## Acceptance criteria

- `Formatter::fmt_stmt` emits a canonical representation of `Node::Extern`
  blocks, including:
  - `extern "<library>" {` header with brace on same line.
  - One declaration per line, indented 4 spaces.
  - Each declaration in the form `[@trusted] fn <alias>(<params>) [-> <ret>][= "<symbol>"];`.
  - `ensures` and `requires` clauses indented under the signature (same
    style as function contracts).
  - Closing `};`.
- `Formatter::fmt_expr`'s `Node::Extern` fallback arm delegates to
  `fmt_stmt` rather than emitting nothing.
- New unit test in `formatter.rs` asserts that an extern block round-trips
  through `fmt` without data loss.
- Golden sidecar `resilient/examples/ffi_libm.expected.txt` (if it exists)
  must not regress.
- `cargo fmt --check` and `cargo clippy -- -D warnings` remain clean.
- Commit: `RES-254: formatter — emit extern blocks instead of silently dropping them`.

## Notes

- `Node::Extern` is defined in `resilient/src/main.rs`; check the variant
  fields (`library: String`, `decls: Vec<ExternDecl>`) to understand the
  data model.
- `ExternDecl` carries `resilient_name`, `foreign_name`, `params`, `ret`,
  `requires`, `ensures`, `trusted` — all need to be emitted.
- This is a subset of RES-197 (full formatter overhaul) but can land
  independently since the formatter infrastructure already exists.
- The `ffi_libm.res` example uses `extern "libm.dylib"` / `"libm.so.6"`;
  the formatter should emit the library string as-is.

## Log

- 2026-04-20 created by analyzer (formatter.rs line 357-358 found during code review)
