---
id: RES-278
title: "Board hygiene: close stale RES-253 — its acceptance criteria are already met"
state: DONE
priority: P2
goalpost: G12
created: 2026-04-20
owner: executor
---

## Summary

`RES-253` ("Board hygiene: close stale IN_PROGRESS entries for RES-133 and
RES-161") is open in `OPEN/`, but its entire acceptance criteria have already
been satisfied: commit `bcdbb6f` moved `RES-133-assume-annotation.md` and
`RES-161-match-bind-subpattern.md` from `IN_PROGRESS/` to `DONE/` on 2026-04-20.

Both ticket files now live in `.board/tickets/DONE/`:
- `DONE/RES-133-assume-annotation.md`
- `DONE/RES-161-match-bind-subpattern.md`

Neither file appears anywhere in `IN_PROGRESS/` today. RES-253 is therefore a
stale open ticket whose work is done.

## Evidence

```
$ ls .board/tickets/IN_PROGRESS/
RES-230-remaining-parser-unwrap-panics.md
RES-243-imports-test-flaky-fixed-tempfile-race.md
...

$ ls .board/tickets/DONE/ | grep "133\|161"
RES-133-assume-annotation.md
RES-161-match-bind-subpattern.md
```

## Acceptance criteria

- `OPEN/RES-253-close-stale-in-progress-133-161.md` is moved to
  `DONE/RES-253-close-stale-in-progress-133-161.md`.
- The file header is updated with `state: DONE` and the closing commit hash.
- `OPEN/RES-278-close-stale-res-253-board-hygiene.md` (this file) is moved
  to `DONE/` with the same commit.
- No other board or source changes are required.
- Commit: `RES-278: close stale RES-253 — acceptance criteria already met`.

## Notes

- This is a pure board-hygiene commit. No source code changes.
- The root cause of this class of problem (stale open tickets whose conditions
  are met before the ticket is explicitly closed) is tracked in RES-244.

## Log

- 2026-04-20 created by analyzer (RES-253 open in OPEN/ but RES-133 and RES-161
  are already in DONE/ as of commit bcdbb6f; criteria satisfied, ticket stale)
closed-by: RES-253 already closed in commit f43c882
