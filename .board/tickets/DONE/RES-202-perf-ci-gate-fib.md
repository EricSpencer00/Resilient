---
id: RES-202
title: Performance CI gate: fib(25) regression blocker
state: DONE
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

## Resolution

### Files added
- `benchmarks/fib/run.sh` — hyperfine driver. Builds
  `resilient` + `resilient-with-jit` (release), then runs
  hyperfine over 10 samples × 3 warmups for each backend
  (walker / VM / JIT) with a pinned RNG seed, extracts the
  medians via `jq`, and emits one JSON object on stdout:
  ```
  { "walker_median_ms": ...,
    "vm_median_ms": ...,
    "jit_median_ms": ...,
    "system": "...", "date": "..." }
  ```
- `benchmarks/baseline/fib.json` — the committed baseline
  (ARM-based dev host; CI's numbers will differ and should
  drive a Manager-reviewed baseline update at first run).
- `scripts/compare_perf.sh` — reads two JSONs, prints a
  markdown table with per-backend deltas, exits 0 if all
  medians are within `PERF_THRESHOLD_PCT` (default 15), 1
  otherwise. Output is suitable for a PR comment.
- `scripts/update_perf_baseline.sh` — regenerates
  `benchmarks/baseline/fib.json` from a fresh local run.
  Prints the prior + new numbers and a reminder to review
  before committing. Human-gated per the ticket.
- `.github/workflows/perf_gate.yml` — runs on push-to-main
  and on PR when any of `resilient/src/**`, `benchmarks/**`,
  `scripts/compare_perf.sh`, or the workflow itself changes.
  Installs `hyperfine` + `jq` + `bc`, runs the benchmark,
  calls `compare_perf.sh`, and posts (or updates) a PR
  comment with the deltas via `actions/github-script`. Fails
  when `compare_perf.sh` exits non-zero.

### Behaviour
- **Threshold**: `PERF_THRESHOLD_PCT` env var at the workflow
  level, default `15` per the ticket. Matches hosted-runner
  variance per the ticket's Notes.
- **PR comment**: the action finds an existing bot comment
  that starts with `### fib(25) perf-gate` and updates it
  instead of posting a fresh one per push; keeps PRs tidy.
- **Failure mode**: `compare_perf.sh` returns exit 1 on
  regression. The workflow echoes the markdown table to the
  log AND publishes it as the PR comment even under `always()`
  — the comment body itself says `**FAIL**`, so the gate is
  visible even when the action itself is red.

### End-to-end local check
- `./benchmarks/fib/run.sh > /tmp/fresh.json` produces a
  valid JSON object (`jq .` parses clean).
- `./scripts/compare_perf.sh benchmarks/baseline/fib.json
  /tmp/fresh.json` on consecutive runs reports `PASS` with
  <5% deltas for the walker + VM rows. JIT has higher
  variance (sub-5ms hyperfine precision warning) but stays
  within the 15% threshold under normal conditions.
- Fabricated-regression test: passing a fresh JSON with 20-67%
  deltas correctly produces `**FAIL**` and exit 1.
- `cargo test --locked` unchanged — the benchmarks directory
  doesn't affect the Rust crate build.

### Design deviations from the AC
- The ticket says "fails if any median is > 15% slower than
  baseline. Configurable via env var." ✓ (`PERF_THRESHOLD_PCT`)
- "On failure, the workflow posts a PR comment with the
  delta." ✓ (the comment is posted unconditionally; the
  comment body carries PASS/FAIL so the signal is visible).
  The ticket arguably implies "only on failure"; posting
  always is friendlier — users see the green-light confirm,
  and drift tracking stays on the PR.
- "To update the baseline, a script … re-runs locally and
  writes the new JSON. Commit-gated by human review." ✓
  — `scripts/update_perf_baseline.sh` writes the JSON and
  prints explicit reminders; the actual `git commit` is a
  human action.

### Known caveats
- **Hosted-runner variance**. The baseline committed here was
  generated on a developer ARM-based macOS host. GitHub
  hosted Ubuntu runners will likely show materially different
  absolute numbers (slower CPU, noisier). First CI run will
  fail the threshold — expected — and the manager should
  run `scripts/update_perf_baseline.sh` against a hosted
  runner (or re-run the CI job to capture numbers, then
  hand-write the baseline). Noted explicitly because the
  ticket's own Notes acknowledge the hosted-runner noise.
- **Hyperfine sub-5ms warning on JIT.** The JIT row runs fib(25)
  in ~2-6ms, which dips below hyperfine's shell-startup
  precision (the tool emits a warning). We keep the row in
  the baseline anyway so regressions >15% still trip the
  gate; for absolute JIT perf numbers use `cargo bench` or a
  criterion-style harness (separate follow-up).

### Follow-ups (not in this ticket)
- **Tighten the threshold** once two-three months of hosted-
  runner data is in. The ticket's Notes flag this.
- **More bench targets** (`sum`, `contracts`, `lex`) could hook
  into the same gate infrastructure — add `run.sh` per bench
  dir and a matrix in the workflow.
- **Criterion harness** for the JIT row to get sub-millisecond
  precision.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (fib(25) perf-gate: hyperfine
  driver + baseline + compare + update scripts + CI workflow
  with PR-comment feedback)
