---
id: RES-121
title: Constraint-based unification with occurs check, split from infer prototype
state: OPEN
priority: P2
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
RES-120 inlines unification inside the inference walker. That's fine
for the prototype, but the unify routine is hot and deserves its own
module with its own tests — especially the occurs check, which is
the thing that prevents `t = List<t>` style runaway expansion once
we add generics in RES-124.

## Acceptance criteria
- `resilient/src/unify.rs` exports:
  - `struct Substitution(HashMap<u32, Type>)`.
  - `fn unify(&mut self, a: &Type, b: &Type) -> Result<(), UnifyError>`.
  - `fn apply(&self, ty: &Type) -> Type`.
- Occurs check: attempting to bind `Var(v)` to a type containing
  `Var(v)` returns `UnifyError::Occurs(v, Type)`.
- Unit tests: basic prim ↔ prim success, prim ↔ prim failure, var
  unified to prim, var unified to var, occurs-check failure on
  `Var(0) = Var(0) → Int` style constructed type.
- `infer.rs` refactored to use the new module; no behavior change
  from RES-120.
- Commit message: `RES-121: split unify() + occurs check into own module`.

## Notes
- `Substitution::apply` must be idempotent — applying twice equals
  applying once. The unit test explicitly asserts this on a
  three-variable chain.
- Composition order for `Substitution::compose(&other)`: "apply other
  first, then self" — the order that makes unify-walks correct.
  Document inline.

## Log
- 2026-04-17 created by manager
