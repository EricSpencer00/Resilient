---
id: RES-244
title: "Board hygiene: recurring ticket ID collisions from concurrent analyzer runs"
state: OPEN
priority: P3
goalpost: G12
created: 2026-04-20
owner: executor
---

## Summary

Multiple analyzer runs on 2026-04-20 created tickets with conflicting IDs.
The `.board/tickets/` tree currently has **three** files claiming `id: RES-239`
and **two** files claiming `id: RES-240`. This breaks any tooling that
relies on IDs being unique and creates confusion for executors.

## Duplicate ticket inventory

### RES-239 (three files)

| File | State | Topic |
|---|---|---|
| `OPEN/RES-239-lint-passes-skip-impl-block-methods.md` | OPEN | Lint passes skip `impl` block methods (L0001–L0005) |
| `IN_PROGRESS/RES-239-imports-missing-file-test-regression.md` | IN_PROGRESS | Imports smoke test failure (bad root-cause analysis — see RES-241) |
| `IN_PROGRESS/RES-239-redundant-closure-lint-src-lint-rs.md` | IN_PROGRESS | Redundant closure in `lint.rs:319` — **already fixed** (clippy passes) |

### RES-240 (two files)

| File | State | Topic |
|---|---|---|
| `OPEN/RES-240-file-io-demo-golden-test-regression.md` | OPEN | `file_io_demo` golden test regression — **already fixed** (golden tests pass) |
| `IN_PROGRESS/RES-240-clippy-redundant-closure-lint-rs.md` | IN_PROGRESS | Same clippy issue as the IN_PROGRESS RES-239 entry — **already fixed** |

## Required cleanup actions

1. **Close `IN_PROGRESS/RES-239-redundant-closure-lint-src-lint-rs.md`** — move
   to `DONE/` with a note that the clippy issue was resolved (clippy is clean).

2. **Close `IN_PROGRESS/RES-240-clippy-redundant-closure-lint-rs.md`** — same
   reason; duplicate of the above, also already fixed.

3. **Close `OPEN/RES-240-file-io-demo-golden-test-regression.md`** — move to
   `DONE/` with a note that `golden_outputs_match` passes and `file_io_demo`
   produces the correct output.

4. **Retain `OPEN/RES-239-lint-passes-skip-impl-block-methods.md`** unchanged —
   this is a legitimate open work item.

5. **Retain `IN_PROGRESS/RES-239-imports-missing-file-test-regression.md`** for
   now (it tracks a real failing test) but note that the root-cause analysis
   is superseded by RES-241.

After cleanup there should be exactly one file per ID in the board.

## Acceptance criteria

- No two ticket files share the same `id:` value.
- Stale/fixed tickets in `OPEN/` or `IN_PROGRESS/` are moved to `DONE/`
  with a closing note.
- `cargo test` and `cargo clippy` remain green.
- Commit: `RES-244: close stale duplicate tickets and fix recurring ID collision issue`.

## Affected paths

- `.board/tickets/IN_PROGRESS/RES-239-redundant-closure-lint-src-lint-rs.md`
- `.board/tickets/IN_PROGRESS/RES-240-clippy-redundant-closure-lint-rs.md`
- `.board/tickets/OPEN/RES-240-file-io-demo-golden-test-regression.md`

## Log

- 2026-04-20 created by analyzer (ID collision found during ticket scan)
