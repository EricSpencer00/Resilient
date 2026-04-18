---
id: RES-129
title: Match exhaustiveness warnings for integer range patterns
state: OPEN
priority: P3
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
RES-054 shipped basic match exhaustiveness for enum-like domains.
Integer matching is currently always "exhaustive via `_`", which
robs us of the safety we get on enums. Teach the checker about
disjoint integer ranges: `1..=3 | 4..=6` with no default is a
warning, not an error — but *should* produce a "missing: 0, 7..`
note.

## Acceptance criteria
- Pattern grammar extended: `N..=M` and `N..M` patterns.
- Exhaustiveness algorithm treats integer patterns as intervals
  over `i64`; uses an interval-coalescing pass to compute the
  uncovered set.
- When a match has no `_` arm and the coalesced cover is proper
  subset of `i64`, emit a warning with a note listing up to 3
  representative uncovered integers (`missing: 0, 7, i64::MAX`).
- New unit tests covering: fully covered disjoint ranges = no
  diagnostic, missing low end, missing high end, missing middle
  gap.
- Commit message: `RES-129: exhaustiveness for integer range patterns`.

## Notes
- Don't turn this into an error — breaking builds for
  unreachable-in-practice integers (`i64::MIN` and friends) is
  annoying. Warning + note is the right friction level.
- The same algorithm generalizes to bool and char ranges; leave
  those for follow-ups.

## Log
- 2026-04-17 created by manager
