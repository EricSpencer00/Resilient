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
- 2026-04-17 claimed and bailed by executor (multi-piece + needs
  warning channel + ticket premise disagrees with current code)

## Attempt 1 failed

Two stacked problems.

1. **Five independent pieces bundled.**
   - Parser: `N..=M` and `N..M` as pattern syntax — disambiguated
     from integer-literal-followed-by-range-operator that appears
     elsewhere.
   - AST: new `Pattern::Range(i64, i64, bool_inclusive)` variant;
     every `Pattern` match in interpreter + typechecker grows an
     arm.
   - Interpreter: range-aware matching for int scrutinees.
   - Typechecker: interval coalescing over i64 + missing-set
     computation (cap at 3 representatives) + warning emission.
     The warning-emission path is entirely new infrastructure —
     today the typechecker only produces errors
     (`Result<Type, String>`); there is no warning channel.
   - Existing-test migration: `typecheck_rejects_int_match_
     without_default` asserts int-without-`_` is an ERROR
     ("Non-exhaustive match on int"). The ticket downgrades that
     to a warning — existing tests need to flip assertion shape.
2. **Missing warning channel.** `TypeChecker::check_program`
   returns `Result<Type, String>`; nothing else rides alongside.
   Adding a warning stream is its own infrastructure ticket
   (probably shares shape with RES-119's bailed Diagnostic
   work). A `print!(...)` to stderr from inside the typechecker
   is a workaround, not a solution — it can't be tested,
   suppressed, or routed into the LSP diagnostic stream.

The ticket's premise — "integer matching is currently always
'exhaustive via `_`'" — is also inaccurate vs. the actual code
on main: int match without `_` is currently a type ERROR, not a
silent accept. The reality is stricter than the ticket assumes,
which means "add the missing check" isn't the work — "relax the
existing check, replace with a warning, and refine it with
intervals" is.

## Clarification needed

Manager, please sequence:

- RES-XXX-a (new): typechecker warning channel.
  `check_program_with_source` returns `(Result<Type, String>,
  Vec<Warning>)` or equivalent. Tests can assert on warning
  shape; the LSP can route warnings through the existing
  publish_diagnostics stream.
- RES-129a: `Pattern::Range` + parser + interpreter match-eval.
  Small, self-contained slice proving a range pattern matches.
- RES-129b: interval coalescing + missing-set computation +
  warning emission via RES-XXX-a's channel. Migrates the
  existing int-match error test to assert warning (not error) +
  the `missing:` note shape the ticket specifies.

No code changes landed — only the ticket state toggle and this
clarification note.
