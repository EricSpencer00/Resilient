---
id: RES-202
title: Performance CI gate: fib(25) regression blocker
state: OPEN
priority: P3
goalpost: perf
created: 2026-04-17
owner: executor
---

## Summary
RES-082 / RES-106 established our flagship microbenches (bytecode
VM and JIT on fib). Without a CI gate, a regression can land
silently. Add a workflow that runs the bench, compares to a
stored baseline, and fails if the regression exceeds a
configurable threshold.

## Acceptance criteria
- `benchmarks/baseline/fib.json` checked in with the current
  results (VM median, JIT median, walker median).
- `.github/workflows/perf_gate.yml` runs `benchmarks/fib/run.sh`,
  extracts medians, compares.
- Threshold: fail if any median is > 15% slower than baseline.
  Configurable via env var.
- On failure, the workflow posts a PR comment with the delta.
- To update the baseline, a script
  `scripts/update_perf_baseline.sh` re-runs locally and writes
  the new JSON. Commit-gated by human review.
- Commit message: `RES-202: fib perf CI gate`.

## Notes
- GitHub hosted runners are noisy. 15% accommodates that without
  being useless. Historical data will let us tighten.
- Don't block merges on an isolated failure — the gate is a
  warning label visible on the PR. Branch-protection configuration
  to enforce is a separate decision.

## Log
- 2026-04-17 created by manager
