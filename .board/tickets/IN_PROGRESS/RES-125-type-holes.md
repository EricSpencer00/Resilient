---
id: RES-125
title: Type holes `_` in annotations are treated as inference placeholders
state: IN_PROGRESS
priority: P3
goalpost: G7
created: 2026-04-17
owner: executor
Claimed-by: Claude
---

## Summary
`let x: _ = 3 + 2` should parse and infer `x: Int`. Useful when the
user wants to assert "there IS a type, I just don't want to write
it" — especially in tutorials. Implementation is trivial once HM is
in: a `_` in a type position becomes a fresh `Type::Var`.

## Acceptance criteria
- Parser accepts `_` as a type: `let x: _`, `fn foo(_ x) -> _`,
  `Array<_>`, etc.
- Each `_` becomes a distinct fresh inference variable; they don't
  unify just because they share a syntactic `_`.
- Display: when the inferer reports an error involving a `_`-origin
  variable, the message reads `type hole at line:col` rather than
  `?t0`.
- Unit tests covering each position (let, param, return, generic
  arg).
- Commit message: `RES-125: `_` type holes as inference placeholders`.

## Notes
- Parameter-position `_` means "infer", not "don't care about the
  value" — don't confuse with pattern holes in `match`.
- If full inference can't pin the hole down at use sites, the
  error message should say `cannot infer type of hole at L:C` with
  the concrete span.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (blocked on RES-120)
- 2026-04-20 claimed by Claude — RES-120 is now done; implementing

## Attempt 1 failed

Blocked on RES-120 (HM inference prototype). The ticket's own
summary says "Implementation is trivial once HM is in: a `_` in
a type position becomes a fresh `Type::Var`." RES-120 is OPEN
with a `## Clarification needed` note (blocked on RES-119 +
missing NodeId); there is no inferer to turn a `_` in a type
position into a fresh `Type::Var` against.

Parser-side alone would be easy to land today (accept `_` and
stash an `Option<Type::Var>` placeholder), but without the
inferer the parser placeholder would go unconsumed — every
typechecker call site that looks at the annotation would need a
`// TODO: infer` fall-through, effectively rolling out RES-120
piecewise. That's the opposite of what the ticket's notes say
about keeping this minimal.

The error-shape work ("cannot infer type of hole at L:C",
"type hole at line:col" instead of `?t0`) also doesn't exist
without an inferer emitting hole-origin type variables.

## Clarification needed

Re-open once RES-120 lands. At that point RES-125 reduces to:

- Parser arm that treats `_` as a type name and records a span.
- At inference time, mint a fresh `Type::Var` tagged with the
  recorded span.
- Display: when an unresolved type variable's origin is a `_`
  hole, render `type hole at L:C` instead of `?tN`.

No code changes landed — only the ticket state toggle and this
clarification note.
