---
id: RES-150
title: `random()` builtin with deterministic `--seed` flag
state: IN_PROGRESS
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
