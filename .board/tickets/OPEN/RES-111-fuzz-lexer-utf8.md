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
- 2026-04-17 claimed and bailed by executor (bin→lib refactor +
  nightly + new CI workflow = 4 iteration-sized pieces)

## Attempt 1 failed

Bailing: four independently-sized pieces bundled.

1. **bin→lib refactor.** cargo-fuzz targets live in a separate
   crate (`fuzz/`) and import from the library; today `resilient`
   is bin-only (`src/main.rs` holds the `Lexer` / `Token`
   definitions). The ticket's `Lexer::new(input).lex()` expects
   a library surface that doesn't exist. Adding `src/lib.rs`
   touches a lot of code paths and is its own ticket.
2. **cargo-fuzz scaffolding** at a fresh `fuzz/` dir.
3. **Nightly toolchain** — cargo-fuzz requires nightly. No
   `rust-toolchain.toml` on `main` today; adding one affects
   every `cargo` invocation in the repo.
4. **New CI workflow** (`.github/workflows/fuzz.yml`) —
   `workflow_dispatch`, 60 s timebox, nightly install, `cargo
   install cargo-fuzz` step.

Plus: any crashes found by the first local run reduce to unit
tests. That's post-discovery work, not upfront, but the policy
needs documenting.

## Clarification needed

Manager, please split:

- **RES-111a: library surface.** Expose `resilient/src/lib.rs`
  with the minimum `pub use` re-exports needed by downstream
  fuzz / bench / LSP integrations (`Lexer`, `Token`, `parse`,
  `typechecker`). Unblocks this ticket *and* the shared-LSP-
  infra rework flagged by RES-181 / RES-182 / RES-188.
- RES-111b: `cargo fuzz init` scaffold + `lex` target using the
  library from 111a. Local smoke: `cargo +nightly fuzz run lex
  -- -max_total_time=10` clean.
- RES-111c: `.github/workflows/fuzz.yml` — `workflow_dispatch`
  only, nightly install, 60 s timebox.
- RES-111d (conditional): any crashes found reduce to a unit
  test + fix commit, one ticket per finding.

Landing 111a first is the leverage point — it unblocks the fuzz
work here and the LSP tickets already waiting on a library
surface.

No code changes landed — only the ticket state toggle and this
clarification note.
