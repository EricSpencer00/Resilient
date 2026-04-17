---
id: RES-192
title: IO-effect inference: flag functions that reach `println` or file_*
state: OPEN
priority: P3
goalpost: G18
created: 2026-04-17
owner: executor
---

## Summary
`@pure` (RES-191) is opt-in and checked. Going further: infer an
effect set for every fn. The MVP tracks one effect — `IO` — and
colors every transitively-IO fn. User-facing surface is an
LSP inlay hint and `--audit` column.

## Acceptance criteria
- Effect lattice for now: `{}` (pure) or `{IO}`. Operators are
  union.
- Pass: fixpoint over the call graph. Builtins pre-populated (from
  RES-191's table). User fns aggregate effects of their body.
- Reported via:
  - `--audit` gains an "effects" column.
  - LSP hover (extension of RES-181) appends
    `[effects: IO]` when non-empty.
- Unit tests: a chain `caller -> helper -> println` tagged IO at
  every step; a leaf fn that only does arithmetic tagged pure.
- Commit message: `RES-192: IO effect inference`.

## Notes
- Keep the lattice small (binary). Adding more effects (MEM,
  ALLOC, PANIC) is follow-up work and requires careful user-level
  documentation.
- Don't error on IO — just report. `@pure` is the error path; this
  is informational.

## Log
- 2026-04-17 created by manager
