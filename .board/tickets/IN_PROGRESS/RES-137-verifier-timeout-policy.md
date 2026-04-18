---
id: RES-137
title: Verifier timeout + soft-failure policy
state: OPEN
priority: P3
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
Z3 can spin indefinitely on certain obligations (QF_NIA is
undecidable). Today we wait. Set a hard per-obligation timeout
(default 5s), treat `unknown`/`timeout` as "not proven" rather than
error, and keep compilation going.

## Acceptance criteria
- CLI flag: `--verifier-timeout-ms <N>` (default 5000).
- Programmatic: pass `timeout` in the Z3 params dict before each
  `solver.check()`.
- On timeout/unknown: emit a *hint*-severity diagnostic
  `proof timed out after 5000ms — runtime check retained` with the
  obligation span. Compilation continues; runtime check is not
  elided.
- `--audit` tallies `timed-out` as its own column.
- Unit test: construct an obviously hard NIA obligation and
  confirm timeout triggers within the budget.
- Commit message: `RES-137: verifier timeout + soft-failure`.

## Notes
- Z3 `timeout` is per-query, not cumulative. Per-fn wall-clock
  budget is a future ticket if needed.
- The hint severity is important: errors would block builds on
  machines with slow Z3 builds (ARM mac, etc.).

## Log
- 2026-04-17 created by manager
