---
id: RES-179
title: Code-size budget CI gate via `cargo bloat` on the cortex-m demo
state: OPEN
priority: P3
goalpost: G16
created: 2026-04-17
owner: executor
---

## Summary
The Cortex-M demo crate (RES-101) is a canary for how heavy the
runtime is in a real embedded build. A CI gate that fails if the
release `.text` section exceeds a budget (say 64 KiB for the
demo's minimal surface) stops surprise regressions from landing.

## Acceptance criteria
- `.github/workflows/size_gate.yml`:
  - Builds `resilient-runtime-cortex-m-demo` release for
    thumbv7em-none-eabihf.
  - Runs `cargo bloat --release --target thumbv7em-none-eabihf`.
  - Extracts the total `.text` size.
  - Fails if > 64 KiB (configurable via a workflow env var).
- On failure, the workflow prints the top 20 largest symbols so
  the regression is attributable.
- README or crate docs record the current measurement + target.
- Commit message: `RES-179: code-size budget CI gate`.

## Notes
- Don't set the budget aggressively — start at 64 KiB (generous).
  Tighten in a follow-up once we have a stable baseline.
- Use `cargo bloat --release --filter ''` to skip the long
  per-file breakdown; grab the total from the first line of
  output.

## Log
- 2026-04-17 created by manager
