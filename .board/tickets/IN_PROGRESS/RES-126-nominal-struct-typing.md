---
id: RES-126
title: Nominal struct equivalence (no accidental structural collapse)
state: OPEN
priority: P3
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
Two structs with identical field sets must be different types. In
a safety-critical context we absolutely do not want
`struct Meters{ val: Int }` and `struct Seconds{ val: Int }` to
unify just because they have the same shape. Verify the
typechecker already does this — and pin it with a test — or fix
if not.

## Acceptance criteria
- New test `nominal_distinct_empty_braces` creates two zero-field
  structs `A` and `B` and asserts an assignment `let a: A = B {};`
  is a type error (with span on `B {}`).
- Test `nominal_distinct_same_shape` does the same for two
  single-field structs with the same field name + type.
- If the existing code already enforces this (likely — RES-038
  introduced structs), just add the tests. If not, update the
  unification rule for `Type::Struct(name, fields)` to compare
  `name` identity, not field equality.
- No SYNTAX.md change — the spec was always nominal; this is
  just regression insurance.
- Commit message: `RES-126: pin nominal-struct distinctness with tests`.

## Notes
- "Newtype" ergonomics (wrapping Int in Meters) is a follow-up:
  today you'd write `let m = Meters { val: 5 }` and reach in via
  `.val`. Sugar for that (e.g. `type Meters = Int(nominal)`) is
  out of scope.

## Log
- 2026-04-17 created by manager
