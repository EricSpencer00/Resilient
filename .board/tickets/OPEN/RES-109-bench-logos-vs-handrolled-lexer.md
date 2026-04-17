---
id: RES-109
title: Benchmark logos lexer vs hand-rolled on a 100 KLoC synthetic input
state: OPEN
priority: P3
goalpost: G5
created: 2026-04-17
owner: executor
---

## Summary
RES-108 lands logos behind a feature flag. Before we cut the default
lexer over, we need numbers. Build a synthetic 100 KLoC Resilient
program (repeated example bodies) and measure scan time on both
paths. If logos isn't ≥2× faster at equal correctness, we keep the
hand-rolled lexer as default and close G5 as "evaluated, declined".

## Acceptance criteria
- New bench at `benchmarks/lex/` with a `run.sh` that:
  - Generates a 100 KLoC `.rs` file by concatenating existing
    examples with renamed identifiers.
  - Runs both lexers via `cargo run --release --bin lex-bench` 100×
    and records p50 / p99 timings.
- `benchmarks/lex/RESULTS.md` records the numbers plus the machine
  spec. Gets committed alongside.
- Decision line at the top of the results doc: "logos keeps / logos
  drops" — manager will update the roadmap from that.
- No changes to default feature flags yet — decision lives in
  RESULTS.md until RES-110 commits to the migration.
- Commit message: `RES-109: bench logos vs hand-rolled lexer`.

## Notes
- Use `std::time::Instant` with wall-clock; we aren't trying to
  beat criterion-grade rigor here, just get the ratio.
- Warm up 5× before the 100× timed loop to avoid first-scan cost.
- Don't include parser / typechecker cost — strictly token stream
  consumed and discarded.

## Log
- 2026-04-17 created by manager
