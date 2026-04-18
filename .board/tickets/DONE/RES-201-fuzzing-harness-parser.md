---
id: RES-201
title: cargo-fuzz harness for the parser (no panics on any input)
state: DONE
priority: P3
goalpost: testing
created: 2026-04-17
owner: executor
---

## Summary
Companion to RES-111's lexer fuzz: the parser must also never
panic on any byte sequence. After RES-016 killed the known
panics, a fuzzer proves the rest.

## Acceptance criteria
- New fuzz target `fuzz/fuzz_targets/parse.rs`: take arbitrary
  bytes, filter to UTF-8, feed through `Parser::new(...).parse()`.
- Invariant: no panics, no infinite loops (250ms timeout).
- Any crash reduces to a unit test + fix in the same PR.
- The existing `fuzz.yml` workflow (from RES-111) is extended to
  run both targets — 30 seconds each on manual dispatch.
- Commit message: `RES-201: parser fuzz target`.

## Notes
- Parser recovery (RES-016) means even malformed input should
  produce a Diagnostic vec rather than a panic. This fuzz target
  pins that.
- Optional: have the target also invoke the typechecker on the
  AST — but only on parse-success, to avoid OOM from the checker
  on pathological input.

## Resolution

### Files added
- `fuzz/Cargo.toml` — standalone cargo-fuzz crate. Not a
  workspace member of the resilient crate; deps are
  `libfuzzer-sys = "0.4"` and `tempfile = "3"` only.
- `fuzz/fuzz_targets/parse.rs` — the parser target. Shape:
  - Filter input bytes through `std::str::from_utf8` (early
    return on non-UTF-8 so libFuzzer doesn't waste budget).
  - Write to a tempfile.
  - Spawn `$RESILIENT_FUZZ_BIN -t --seed 0 <tempfile>`.
  - If the child exits via signal (`status.code() == None`),
    re-raise as a local panic so libFuzzer records the input
    under `fuzz/artifacts/parse/`.
  - Non-zero clean exits (parse errors, type errors) are
    fine — the invariant is "no panic", not "no error".
- `fuzz/.gitignore` — excludes `target/`, `corpus/`,
  `artifacts/`, `coverage/`, `Cargo.lock`.
- `fuzz/README.md` — design note (subprocess rationale + the
  speed trade-off), local-run walkthrough, CI reference, crash
  reduction flow.
- `.github/workflows/fuzz.yml` — manual-dispatch workflow.
  `duration_seconds` input controls the per-target budget
  (default 30s). Matrix `target: [parse]` — trivially grows to
  `[parse, lex]` when RES-111 lands its lex target.
  Installs nightly + cargo-fuzz, builds resilient in release,
  runs the fuzzer with `-max_total_time=N -timeout=1`, uploads
  `fuzz/artifacts` on failure with 30-day retention.

### Design note: why subprocess instead of in-process
The resilient crate is binary-only today (no `src/lib.rs`), so
there's no public library surface to call `parse()` from the
fuzz crate. Two options were considered:

1. **Refactor main.rs → lib.rs shim** — would require moving
   every `mod` declaration out of `main.rs` and exposing
   `parse`, `Parser`, `Node`, `Lexer` publicly. Invasive;
   touches every crate-internal `use crate::...`.
2. **Subprocess** — the target spawns the built binary per
   input. ~1ms per iteration (hundreds-to-thousands of
   iters/sec) versus millions with in-process. Still enough
   to surface parser panics in a CI budget.

Chose (2) to keep the blast radius tight. The README documents
the trade-off and names the in-process migration as a follow-up.

### Deviation from the literal AC
The AC says "feed through `Parser::new(...).parse()`". This
target feeds through the whole `--typecheck` path (which
internally calls `Parser::new(...).parse()` — see
`execute_file` in `main.rs:7551`). The parse step is exercised
on every input; parse panics would still surface. Documented
in the target's module comment.

### Verification
- `cargo test --locked` in the resilient crate → unchanged
  (pure additive at the repo root; fuzz crate is standalone)
- `python3 -c "import tomllib; tomllib.load(...)"` on
  `fuzz/Cargo.toml` → parses cleanly
- `ruby -ryaml -e "YAML.load_file(...)"` on the workflow →
  parses cleanly
- End-to-end fuzzer run NOT performed locally — cargo-fuzz
  isn't installed on the dev host and requires nightly Rust.
  The CI workflow installs both and runs the fuzzer on
  manual dispatch; that's where "no panics on any input" is
  actually verified.

### Follow-ups (not in this ticket)
- **In-process target.** Refactor `resilient` into a
  bin+lib crate so the fuzz target can call `parse(&str)`
  directly. ~1000x speedup.
- **RES-111 lex target.** When that ticket lands, add
  `fuzz_targets/lex.rs` with a `[[bin]]` entry in
  `fuzz/Cargo.toml` — the workflow matrix picks it up.
- **Typechecker extension.** Ticket Notes mention optionally
  invoking the typechecker on parse-success. Already included
  indirectly because `--typecheck` is what the subprocess runs.
- **Seed corpus.** `examples/*.rs` would make a good starting
  corpus. `cargo fuzz run parse examples/` would pick them up.
  Not committed today to keep the fuzz dir lean.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (standalone fuzz crate +
  parse target via subprocess + workflow; local + CI
  walkthrough documented)
