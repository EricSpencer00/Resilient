---
id: RES-253
title: "Board hygiene: close stale IN_PROGRESS entries for RES-133 and RES-161"
state: DONE
priority: P1
goalpost: G12
created: 2026-04-20
owner: executor
---

## Summary

PRs #48 and #49 shipped RES-133a (`assume()` parser + AST + runtime) and
RES-161a (`Pattern::Bind`) respectively, but their ticket files were never
moved from `IN_PROGRESS/` to `DONE/`. The stale entries cause confusion
for any executor scanning the board for claimable work.

## Evidence

- `git log --oneline` shows:
  ```
  6ada8e3 RES-133a: assume() — parser, AST, and runtime evaluation (#48)
  94595a5 RES-161a: Pattern::Bind — name @ inner match patterns (#49)
  ```
- Both files remain in `.board/tickets/IN_PROGRESS/`.
- `cargo test` passes green; the features are fully implemented.

## Acceptance criteria

1. `.board/tickets/IN_PROGRESS/RES-133-assume-annotation.md` is moved to
   `DONE/` with a `Closed-at: 6ada8e3` line added to the `## Log` section.
2. `.board/tickets/IN_PROGRESS/RES-161-match-bind-subpattern.md` is moved to
   `DONE/` with a `Closed-at: 94595a5` line added to the `## Log` section.
3. No source changes — board-only cleanup.
4. Commit: `RES-253: close stale IN_PROGRESS entries for RES-133 and RES-161`.

## Notes

- Both tickets are scoped to their `a`-suffix tasks (parser + AST + runtime
  only). The broader acceptance criteria for the full RES-133 / RES-161
  may have remaining sub-tasks, but the implemented scope is done.
- Check whether any downstream tickets block on RES-133 or RES-161 and
  update their `## Notes` if so.

## Log

- 2026-04-20 created by analyzer (stale IN_PROGRESS entries found during board scan)
closed-by: shipped in commit 0552548 (main)
