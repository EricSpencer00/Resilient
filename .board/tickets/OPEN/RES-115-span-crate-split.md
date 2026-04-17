---
id: RES-115
title: Split `span.rs` into its own crate `resilient-span`
state: OPEN
priority: P3
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
`span.rs` is now consumed by four different modules (lexer, parser,
typechecker, LSP) and will soon be consumed by the JIT debug-info
path (RES-167+ range). Keeping it in `resilient/src/` forces those
consumers to live in the same crate. Extracting to
`resilient-span/` (sibling of `resilient-runtime/`) lets future
tooling pick it up without dragging the whole compiler in.

## Acceptance criteria
- New crate `resilient-span/` with `Pos`, `Span`, `Spanned<T>` as
  today, plus a `pos_from_byte` helper (ported from RES-110).
- `resilient/Cargo.toml` adds `resilient-span = { path =
  "../resilient-span" }` and `resilient/src/span.rs` shrinks to a
  `pub use resilient_span::*;` shim.
- `resilient-runtime` does NOT gain this dep (still no_std-clean;
  the span crate is compiler-side only). Verify in CI.
- All four feature configs pass cargo test + clippy.
- Commit message: `RES-115: extract span types to resilient-span crate`.

## Notes
- Don't make the new crate no_std — spans are a compile-time concept
  and compile-time consumers can assume std.
- Keep the module path stable (`crate::span::Span` still resolves)
  so downstream tickets don't thrash on imports.

## Log
- 2026-04-17 created by manager
