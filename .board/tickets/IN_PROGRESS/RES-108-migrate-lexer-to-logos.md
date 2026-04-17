---
id: RES-108
title: Migrate lexer to the `logos` crate (G5 foundation)
state: IN_PROGRESS
priority: P2
goalpost: G5
created: 2026-04-17
owner: executor
---

## Summary
G5 has been parked since session 1: the hand-rolled lexer is solid but
makes every new token a manual edit. Migrating to `logos` gives us a
single derive-macro declaration, faster scanning on large inputs, and
a clean place to attach span metadata. This ticket introduces the
crate behind a feature flag so we can cross-check against the legacy
lexer before cutting over.

## Acceptance criteria
- New dep: `logos = "0.14"` in `resilient/Cargo.toml`.
- New module `resilient/src/lexer_logos.rs` defines a `#[derive(Logos)]`
  enum covering every token variant the current lexer produces.
- Feature flag `logos-lexer` in `Cargo.toml`; when enabled, `main.rs`
  routes `Lexer::new(src)` to the logos path.
- Parity test: a new `lexer_parity` test harness feeds every example
  in `resilient/examples/` into both lexers and asserts the token
  streams (kind + lexeme + span) match exactly.
- `cargo test --features logos-lexer` is clippy clean.
- The legacy hand-rolled lexer stays as the default until RES-109
  benchmarks land.
- Commit message: `RES-108: logos-based lexer behind feature flag`.

## Notes
- Keep `Token` enum stable — logos only replaces the scanner, not the
  downstream parser contract.
- Regex in logos is Rust-regex, not PCRE; check that our numeric
  literal rules (hex `0x`, binary `0b`, digit separators) translate.
- String-literal scanning needs a `callback` to handle escape
  sequences; don't let logos strip them silently.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
