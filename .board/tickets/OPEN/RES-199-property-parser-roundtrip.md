---
id: RES-199
title: Property-based test: `format(parse(src)) == src` for canonical files
state: OPEN
priority: P3
goalpost: testing
created: 2026-04-17
owner: executor
---

## Summary
With the formatter (RES-197) in place, we get a strong invariant
for free: for any source already in canonical form, a
parse + format round trip produces identical output. Use
`proptest` to generate canonical-shape inputs and assert the
property.

## Acceptance criteria
- New dev-dependency: `proptest = "1"`.
- New test module `tests/roundtrip.rs`:
  - Generator for canonical programs (fn decls, expressions, let
    bindings, if/else, while, arrays, structs).
  - Strategy: breadth-limited recursion to keep test time
    manageable.
  - Property: `fmt(parse(fmt(parse(src)))) == fmt(parse(src))`
    (formatter idempotence) AND `format(parse(src)) == src` when
    `src` is already canonical.
- 1000 cases per run by default; configurable via
  `PROPTEST_CASES` env var.
- Shrinking enabled so failures produce minimal counterexamples.
- Commit message: `RES-199: proptest parser / formatter roundtrip`.

## Notes
- Proptest can be flaky under tight CI time budgets. Gate the test
  behind `#[cfg(feature = "proptest")]` + a CI feature flag; run
  on merge, not every PR.
- Shrinking is the highest-leverage feature here — without it,
  counterexamples are noisy.

## Log
- 2026-04-17 created by manager
