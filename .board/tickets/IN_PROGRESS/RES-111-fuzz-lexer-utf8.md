---
id: RES-111
title: Fuzz the lexer with arbitrary UTF-8 input (no panics, ever)
state: OPEN
priority: P3
goalpost: G5
created: 2026-04-17
owner: executor
---

## Summary
RES-016 killed every known parser/lexer panic, but "known" is the
operative word. A fuzzer throws bytes we'd never think to hand-write
and catches the rest. Since we already exit non-zero on error
(RES-027), the property is simple: for any UTF-8 input, the lexer
must either emit a token stream ending in EOF or record a diagnostic
and return an empty stream — never panic, never loop.

## Acceptance criteria
- New `fuzz/` directory at repo root with `cargo-fuzz` scaffolding
  (`cargo fuzz init`).
- Target `fuzz_targets/lex.rs` that calls `Lexer::new(input).lex()`
  on arbitrary UTF-8 bytes.
- Invariant asserted by the target: never panic, no infinite loop
  (wall-clock timeout of 250 ms via `fuzz_target!` config).
- 60-second CI fuzz run in `.github/workflows/fuzz.yml` (manual
  `workflow_dispatch` trigger only — not per-PR; fuzzing is long-tail).
- Any crash found by local `cargo fuzz run lex` reduces to a unit
  test committed in `resilient/src/main.rs` `mod tests` and fixed
  inline in the same PR.
- Commit message: `RES-111: fuzz lex() with cargo-fuzz`.

## Notes
- cargo-fuzz requires nightly Rust; gate the workflow with
  `rustup default nightly` + `cargo install cargo-fuzz` steps.
- Arbitrary bytes ≠ arbitrary UTF-8; wrap the input in
  `std::str::from_utf8(data).ok()?` and return early on non-UTF-8
  so we fuzz only the scanning logic, not the decode step.

## Log
- 2026-04-17 created by manager
