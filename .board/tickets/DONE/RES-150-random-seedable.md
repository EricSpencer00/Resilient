---
id: RES-150
title: `random()` builtin with deterministic `--seed` flag
state: DONE
priority: P3
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
Random numbers are useful for simulation and tests. For
safety-critical code we want determinism front-and-center: every
invocation of the compiler/runtime can be forced to a fixed seed
via `--seed N`, and the default is to print the seed used to stderr
at program start so a failing run can be reproduced.

## Acceptance criteria
- Builtins: `random_int(lo: Int, hi: Int) -> Int` (half-open
  [lo, hi)), `random_float() -> Float` ([0.0, 1.0)).
- PRNG: SplitMix64 — small, deterministic, fast, no deps.
- CLI: `--seed <u64>` pins the seed. Without the flag, seed is
  drawn from `clock_ms()` and logged to stderr on program start as
  `seed=<N>`.
- Unit tests: with fixed seed, the first 10 calls produce a
  specific expected sequence.
- Gate on std. no_std would need a hardware RNG abstraction;
  separate ticket.
- Commit message: `RES-150: seedable random builtins`.

## Notes
- Do not use `rand` crate — adds dep surface for minimal gain at
  the scale of SplitMix64. ~15 LOC of algorithm.
- Do not offer "secure" random — we are not cryptographic.
  Document this loudly in README.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - New `static RNG_STATE: AtomicU64` + `RNG_SEED_USED: AtomicU64`
    — process-wide SplitMix64 state, initialized to a visibly
    synthetic sentinel `0xdead_beef_cafe_f00d` so accidental
    "random before seed" usage is diagnosable.
  - `splitmix64_next()` — 15-line canonical SplitMix64 finalizer
    (three constants, three multiplications). Uses `fetch_add`
    for thread-safe state advancement without a mutex — relaxed
    ordering because the RNG is a consumer, not a sync primitive.
    Zero dependencies per the ticket's Notes.
  - `seed_rng(seed)` and `seed_rng_from_clock()` helpers: the
    latter derives a seed from `CLOCK_EPOCH` (RES-147) mixed
    with `std::process::id()` and Knuth's golden-ratio
    multiplier so two processes started in the same ns window
    still differ.
  - `builtin_random_int(lo, hi) -> Int` — half-open `[lo, hi)`,
    rejects `hi <= lo`. Tiny-bias `u64 % span` mapping (good
    enough for sim / tests, ticket explicitly says "not
    cryptographic").
  - `builtin_random_float() -> Float` — top-53-bits / 2^53 for
    a uniform-over-doubles sample in `[0.0, 1.0)`.
  - Both builtins registered in the `BUILTINS` table.
- `resilient/src/typechecker.rs`: `random_int` as
  `fn(Int, Int) -> Int`, `random_float` as `fn() -> Float`.
- CLI: new `--seed <u64>` (both `--seed N` and `--seed=N`
  forms) parsed alongside the other flags. Without the flag,
  main calls `seed_rng_from_clock()` and echoes `seed=<N>` to
  stderr for reproducibility — silent when the user already
  pinned it.
- `README.md`: new "Randomness (RES-150)" subsection under
  REPL Commands spelling out the SplitMix64 choice, the
  `--seed` / stderr echo contract, and a **loud** non-
  cryptographic disclaimer per the ticket's Notes.
- Deviations: none from the acceptance criteria. std-only;
  no_std hardware-RNG abstraction is deferred as the ticket
  prescribes.
- Unit tests (9 new, plus a shared `RNG_TEST_LOCK: Mutex` so
  tests that assert on exact sequences don't race under
  cargo's default parallel runner):
  - `splitmix64_matches_reference_sequence_for_seed_1` — 10
    exact u64s from seed=1, locking down the constants
  - `random_int_is_deterministic_under_same_seed` — same seed
    twice, same 10-sample sequence
  - `random_int_stays_in_half_open_range` — 200 samples in
    `[10, 20)` over a fixed seed
  - `random_int_rejects_reversed_bounds` (both `hi == lo` and
    `hi < lo`)
  - `random_int_rejects_non_int_args`
  - `random_int_rejects_wrong_arity`
  - `random_float_in_unit_interval` — 200 samples in `[0, 1)`
  - `random_float_rejects_arguments`
  - `seed_rng_pins_subsequent_calls`
- Smoke (manual):
  - `cargo run --seed 42 …` produces a stable sequence
    (413, 291, 858, 764, 250).
  - Without `--seed`, stderr prints e.g. `seed=1570391179253402549`
    and the sequence differs per run.
- Verification:
  - `cargo test --locked` — 386 passed (was 377 before RES-150)
  - `cargo test --locked --features logos-lexer` — 387 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
  - Ran the full suite 5 times to confirm no race-condition
    flakes on the sequence-asserting tests (the `RNG_TEST_LOCK`
    mutex serializes them; independent-sample tests don't need
    the lock since they only assert on bounds).
