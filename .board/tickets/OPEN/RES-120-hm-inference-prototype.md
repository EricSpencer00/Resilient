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
- 2026-04-17 claimed and bailed by executor (blocked — see Attempt 1)

## Attempt 1 failed

Bailing: this ticket is blocked on two missing prerequisites.

1. **`Vec<Diagnostic>` does not exist yet.** The `infer_function`
   signature on line 22 is
   `Result<HashMap<NodeId, Type>, Vec<Diagnostic>>`, and line 33
   requires failure tests to assert on `Diagnostic.span`. Both come
   from RES-119 (Unified `Diagnostic` type), which I bailed the
   previous iteration for an internal scope conflict — no
   `resilient/src/diag.rs` exists yet, and no `Diagnostic` struct.

2. **`NodeId` does not exist.** `grep -n NodeId src/` returns nothing.
   The AST is `Node` / `span::Spanned<Node>`; there is no stable
   id-per-node infrastructure. Building one is its own ticket
   (thread an id counter through the parser, plumb it through
   `Spanned`, decide equality / hashing semantics). The ticket
   silently presupposes this and doesn't name a dep.

## Clarification needed

The Manager needs to either:

- **Gate on RES-119 split + a new NodeId ticket.** Produce RES-119a
  (Diagnostic scaffolding — see RES-119's `## Clarification needed`)
  and a new RES-XXX-nodeid-threading ticket, then re-scope RES-120
  to build on them.
- **Rewrite RES-120 to avoid the missing deps.** Specifically:
  - Replace `Vec<Diagnostic>` in the return type with
    `Result<HashMap<..., Type>, String>` (or `Vec<String>`) for the
    prototype, with a `TODO(RES-119)` to migrate later.
  - Replace `HashMap<NodeId, Type>` with a substitution map keyed by
    type-variable id (`HashMap<u32, Type>`) since the prototype only
    needs to surface the substitution, not index back into the AST.
  - Keep Algorithm W + occurs-check + primitive-only scope.

Option 2 lets this ticket land without waiting on the two upstream
rewrites; option 1 is cleaner long-term. Either is fine.

No code changes landed — only the ticket state toggle and this
clarification note. Committing the bail as a ticket-only move so
`main` is unchanged except the metadata.
