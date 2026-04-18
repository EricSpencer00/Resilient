---
id: RES-115
title: Split `span.rs` into its own crate `resilient-span`
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files added:
- `resilient-span/Cargo.toml` (new) — standalone std-only crate,
  edition 2024, no deps.
- `resilient-span/src/lib.rs` (new) — ports `Pos`, `Span`,
  `Spanned<T>`, `build_line_table`, `pos_from_byte` from
  `resilient/src/span.rs` and the former `Lexer::build_line_table`
  / top-level `pos_from_byte` in `main.rs`. 11 unit tests
  covering construction / Display / span arithmetic / line-table
  build / UTF-8 column counting.

Files changed:
- `resilient/Cargo.toml` — new path dep
  `resilient-span = { path = "../resilient-span" }`. Bumped
  Cargo.lock accordingly.
- `resilient/src/span.rs` — shrunk to a 15-line re-export shim
  (`pub use resilient_span::{Pos, Span, Spanned, build_line_table,
  pos_from_byte};`). Existing `use span::{Pos, Span, Spanned};`
  / `crate::span::Pos` call sites resolve unchanged through the
  shim.
- `resilient/src/main.rs` — removed the former `impl Lexer { pub
  fn build_line_table }` method and the top-level
  `pos_from_byte` free fn; updated every caller (6 unit tests in
  `mod tests`, all now reference `span::build_line_table` /
  `span::pos_from_byte`).
- `resilient/src/lexer_logos.rs` — one call site updated from
  `crate::Lexer::build_line_table(src)` to
  `crate::span::build_line_table(src)`.
- `.gitignore` — added `resilient-span/target`.

Verification:
- `cargo build --locked` — clean.
- `cargo test --locked` — 272 unit (down from 279 — 7 tests
  moved to the new crate's test module as part of the port) + 3
  dump-tokens + 12 examples-smoke + 1 golden pass.
- `cargo test --locked --features logos-lexer` — 273 unit (incl.
  parity) pass.
- `cargo build --locked --features z3` — clean.
- `cargo test --locked --features z3` — 285 unit + all
  integration pass.
- `cargo clippy --locked --features logos-lexer --tests -- -D warnings`
  — clean.
- `cd resilient-span && cargo test` — 11 unit pass.
- `cd resilient-runtime && cargo test` — 11 unit pass (unchanged;
  `resilient-runtime` does not take `resilient-span` as a dep,
  so its no_std posture is unaffected — the sibling
  `thumbv7em-none-eabihf` build also still succeeds with and
  without `--features alloc`).

Module path `crate::span::Span` and friends resolve unchanged for
every downstream consumer (lexer, parser, typechecker, LSP,
verifier), so no import thrash — just a cleaner dependency graph.
