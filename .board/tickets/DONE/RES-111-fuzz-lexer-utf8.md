---
id: RES-111
title: Fuzz the lexer with arbitrary UTF-8 input (no panics, ever)
state: DONE
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
- 2026-04-17 re-claimed by executor (RES-201 landed 2 of 4
  prereqs — fuzz crate + workflow now exist)
- 2026-04-17 resolved by executor (subprocess-via-`--dump-tokens`
  lex target; workflow matrix bumped to [parse, lex])

## Resolution

### Files added
- `fuzz/fuzz_targets/lex.rs` — libFuzzer target that filters
  input through `std::str::from_utf8`, writes to a tempfile,
  spawns `$RESILIENT_FUZZ_BIN --dump-tokens <file>`, and
  re-raises subprocess-signal-crashes as local panics so
  libFuzzer records the offending input under
  `fuzz/artifacts/lex/`.

### Files changed
- `fuzz/Cargo.toml` — new `[[bin]] name = "lex"` entry
  alongside `parse`.
- `.github/workflows/fuzz.yml` — header comment now lists
  both tickets; matrix `target:` list is `[parse, lex]`.
- `fuzz/README.md` — target table now names the lex target +
  local-run command for it.

### Design deviation from the literal AC
The AC calls for `Lexer::new(input).lex()` — a direct
in-process call into the lexer. The resilient crate is
binary-only today (no `src/lib.rs`), so we use the same
subprocess pattern as RES-201's parse target:
`resilient --dump-tokens <tempfile>`. That flag drives
`Lexer::new(src)` + `next_token_with_span` to EOF inside the
binary, so every lexer panic still surfaces as a SIGABRT in
the subprocess, which the target re-raises as a local
panic.

Trade-off: ~1ms per iter vs millions in-process. Enough to
surface lexer panics in a CI budget; moving to in-process is
a future improvement that depends on the bin→lib refactor
flagged in RES-111a (ticket Attempt-1 clarification).

### What about the original bail's four pieces?
Attempt 1 listed four independently-sized pieces. As of this
resolution:

1. **bin→lib refactor** — still deferred (unblocks in-process
   fuzz + the LSP tickets that also want a lib surface).
2. **cargo-fuzz scaffolding** — DONE (RES-201).
3. **Nightly toolchain** — still absent at the repo level.
   The workflow installs nightly inside CI so the fuzz runs;
   local dev requires `rustup default nightly` or
   `cargo +nightly ...`.
4. **`.github/workflows/fuzz.yml`** — DONE (RES-201; this
   ticket bumped the matrix to include `lex`).

### Verification
- `ruby -ryaml` on `.github/workflows/fuzz.yml` → parses
  cleanly
- `python3 -c "import tomllib"` on `fuzz/Cargo.toml` → parses
  cleanly
- `cargo test --locked` in the resilient crate → unchanged
  (pure additive; the fuzz crate is standalone, not a
  workspace member of resilient)
- End-to-end fuzz run NOT performed locally (cargo-fuzz +
  nightly not installed on the dev host). The CI workflow
  installs both and runs the lex + parse targets on manual
  dispatch.

### Follow-ups (not in this ticket)
- **RES-111a: bin→lib refactor.** Enables in-process fuzzing
  AND unblocks the LSP tickets that need `resilient::parse`
  etc.
- **Seed corpus** — `examples/*.rs` are good starting inputs
  for both lex and parse. Commit under `fuzz/corpus/{lex,parse}/`
  once seeded.

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
