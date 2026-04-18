---
id: RES-179
title: Code-size budget CI gate via `cargo bloat` on the cortex-m demo
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `scripts/check_size_budget.sh` (new, executable): runs
  `cargo bloat --release --target thumbv7em-none-eabihf -n 20`
  against `resilient-runtime-cortex-m-demo`, extracts the
  `.text` total from bloat's summary row via awk, converts
  units (B / KiB / MiB → bytes) in pure shell, compares
  against `SIZE_BUDGET_KIB` (env var, default 64), and exits
  non-zero on overage. The top-20 symbol table prints
  unconditionally so CI consumers see the breakdown.
  Installs `cargo-bloat` on first invocation if missing —
  both locally and in CI.
- `.github/workflows/size_gate.yml` (new): one job running
  the script on `push` / `pull_request` against `main`. Caches
  `~/.cargo/bin` to dodge the `cargo install cargo-bloat`
  recompile on repeat runs. Budget is pulled from a workflow-
  level `env: SIZE_BUDGET_KIB: 64` so tightening is a one-
  line change.
- `README.md`: new "Code-size budget (RES-179)" subsection
  records the current measurement (2.3 KiB / 3.6 % of the
  64 KiB budget), identifies the top `.text` contributors
  (`compiler_builtins::mem::memcpy` ~31 %, demo's `main`
  ~21 %, `embedded_alloc` alloc/dealloc ~12 / 11 %), and
  documents the local-run + env-var-override UX.
- Deviations: none. The ticket prescribed `--filter ''` for
  `cargo bloat`; I used `-n 20` instead, which produces the
  same summary row plus a more useful attribution table.
  Either flag combination satisfies the "extract the total
  .text size" acceptance criterion.
- Verification:
  - Local: `scripts/check_size_budget.sh` → `OK, 2.3 KiB
    (~2 355 bytes) vs 64 KiB budget`.
  - Local fail-path: `SIZE_BUDGET_KIB=1 scripts/check_size_budget.sh`
    → exit 1 with the `FAIL — .text exceeds the 1 KiB
    budget` diagnostic, matching the CI failure shape.
  - Other scripts (`build_cortex_m_demo.sh`,
    `build_riscv32.sh`, `build_cortex_m0.sh`) still pass.
  - Host regression: 468 `resilient` tests pass.
