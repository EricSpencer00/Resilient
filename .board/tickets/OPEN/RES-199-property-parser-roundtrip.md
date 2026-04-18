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
- 2026-04-17 claimed by executor
- 2026-04-17 claimed and bailed by executor (blocked on RES-197
  formatter — see Attempt 1)

## Attempt 1 failed

Bailing: the properties in the AC are directly about a formatter
that doesn't exist on `main`.

### The dep chain

The ticket's summary names RES-197 (formatter) as the upstream
infrastructure. RES-197 is currently OPEN with a detailed
`## Attempt 1 failed` explaining the ticket is oversized and
bundles four sub-projects (pretty-printer covering 39 AST
variants, comment preservation, `--check`/`--stdin`/recursive-
cwd walk, idempotence harness).

Both load-bearing properties in the AC reference the missing
function:

- `fmt(parse(fmt(parse(src)))) == fmt(parse(src))` — requires
  `fmt`.
- `format(parse(src)) == src` when src is canonical — requires
  `format`.

Without `fmt` / `format`, neither property has a left-hand side
to evaluate. The test module would be stubs.

### What could still land, but shouldn't

A proptest generator for "canonical Resilient programs" is a
useful artifact independently of the formatter — it could feed a
lexer fuzz, a parser fuzz, or future formatter tests. But
landing the generator alone without either of the two target
properties wouldn't satisfy this ticket's AC.

### Clarification needed

Two resequence options for the Manager:

1. **Wait on the RES-197 split.** Once RES-197a (pretty-printer
   scaffolding) lands, RES-199 has something to test. At that
   point RES-199 could reduce scope to just idempotence
   (`fmt(fmt(src)) == fmt(src)`) — the weaker property — which
   lines up with whatever subset RES-197a covers.
2. **Rewrite RES-199 as "parser round-trip via serialized AST".**
   Property: `parse(debug_print(parse(src))) == parse(src)`
   where `debug_print` is a non-canonical Debug-style dump. This
   tests parser stability without depending on a formatter.
   Different property, different name, probably deserves its
   own ticket id.

No code changes in this attempt — only the ticket state toggle
and this clarification note. `main` is unchanged except the
metadata.
