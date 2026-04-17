---
id: RES-122
title: Let-polymorphism for top-level `fn` declarations
state: OPEN
priority: P2
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
Once RES-124 lands `fn<T>`, we'll want a single `id<T>(x: T) -> T`
function callable at both `Int` and `String`. That requires
generalizing over free type variables at binding time and
instantiating fresh ones at each use — classic let-polymorphism.
Scope this to top-level fns; nested lets with generalization is a
rabbit hole we don't need yet.

## Acceptance criteria
- `infer.rs` gains a `generalize(env: &TypeEnv, ty: Type) -> Scheme`
  helper that wraps a `Scheme { vars: Vec<u32>, ty: Type }` with
  quantifiers over `ftv(ty) \ ftv(env)`.
- Top-level fns get their inferred type generalized after body
  inference and stored in the env as a `Scheme`.
- Each call site instantiates the scheme with fresh vars before
  unifying with the argument types.
- Unit tests: `id<T>` called at Int + String in the same program
  succeeds. `fn swap<A,B>(a, b) -> (B, A)` inferred without
  explicit signature.
- Let bindings (non-`fn`) do NOT generalize — keep the value
  restriction trivial (everything's a value, but we just don't
  generalize lets).
- Commit message: `RES-122: top-level let-polymorphism`.

## Notes
- Don't hoist every function into a scheme — only fns with free
  vars after generalization. Monomorphic fns stay monomorphic.
- Error message for ambiguous generalization (rare, but happens
  with `fn foo() { bar }` where `bar` has a free var):
  `cannot generalize: type variable ?0 escapes the let binding`.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (blocked — see Attempt 1)

## Attempt 1 failed

Blocked: this ticket builds on `infer.rs` (from RES-120) and the
`fn<T>` syntax (from RES-124). Neither is in place on `main`.

- RES-120 (HM inference prototype) is currently in OPEN with a
  `## Clarification needed` note (blocked on RES-119's Diagnostic
  scaffolding and an absent NodeId).
- RES-124 (generic `fn<T>` declarations) is further down the queue
  and also depends on RES-120.

The first acceptance criterion reads "`infer.rs` gains a
`generalize(env: &TypeEnv, ty: Type) -> Scheme` helper" — there is
no `infer.rs` on `main` to gain anything.

## Clarification needed

Re-open once RES-120 and RES-124 have landed. Landing this ticket
without them would require building the entire inference walker +
generic-fn parser inside one iteration, which is the opposite of the
ticket's "keep THIS ticket minimal" guidance in similar wording from
RES-120.

No code changes landed — only the ticket state toggle and this
clarification note.
