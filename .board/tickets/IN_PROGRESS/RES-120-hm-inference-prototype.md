---
id: RES-120
title: Hindley-Milner inference prototype over Int / Bool / Float / String
state: OPEN
priority: P2
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
G7 has been waiting for real type inference since session 2.
RES-052/053/054 added a nominal-ish check that rejects obvious
mismatches but doesn't infer. This ticket is the prototype spike:
classic Algorithm W over the primitive monotypes we already have,
scoped to function bodies (no generics, no let-polymorphism yet).

## Acceptance criteria
- New module `resilient/src/infer.rs` feature-gated behind `infer`
  (opt-in; RES-123 flips it on by default once the surface is
  covered).
- `infer_function(func: &Function) -> Result<HashMap<NodeId, Type>, Vec<Diagnostic>>`.
- Unification uses the `Type` enum already in `typechecker.rs`,
  extended with `Type::Var(u32)` for fresh inference variables.
- Literal inference: integer literal → `Type::Int`, float → Float,
  etc. No int↔float coercion; producing a constraint `x : Int`
  from an integer literal is the policy (see RES-130).
- Operator rules hard-coded for the existing operator set (`+`, `-`,
  `*`, `/`, `%`, `&&`, `||`, comparisons). Bitwise ops constrain both
  operands to `Int`.
- A minimal new test suite (`infer_tests.rs`) with ~20 cases covering
  inference success, unification failure, and operator type
  constraints. Each failure case asserts on `Diagnostic.span`.
- Commit message: `RES-120: HM inference prototype (feature=infer)`.

## Notes
- Follow-up tickets (RES-121..125) extend this to let-polymorphism,
  generics, holes, etc. Keep THIS ticket minimal — get the core
  algorithm landed and tested first.
- Occurs-check must be present from day one, even though the
  primitive-only surface can't exercise it — RES-124 needs it.

## Log
- 2026-04-17 created by manager
