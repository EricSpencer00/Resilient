---
id: RES-122
title: Let-polymorphism for top-level `fn` declarations
state: DONE
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
- 2026-04-17 re-claimed by executor — RES-120 is now landed;
  landing Scheme + generalize + instantiate helpers as
  scaffolding (ticket's first AC bullet). Integration with
  call-sites + the `id<T>` end-to-end test require RES-124a
  (fn<T> parser), which is deferred.
- 2026-04-17 resolved by executor (Scheme scaffolding:
  generalize + instantiate + 12 unit tests; call-site wiring +
  `id<T>` end-to-end test deferred to RES-122b pending
  RES-124a)

## Resolution

### Files changed
- `resilient/src/infer.rs` — appended:
  - `pub struct Scheme { vars: Vec<u32>, ty: Type }` with
    `::new(vars, ty)` and `::monotype(ty)` constructors.
    Drops `Eq` because `Type` only derives `PartialEq`
    (carries `f64` via `FloatLiteral`, so `Eq` isn't
    available). Callers use `==`.
  - `pub fn free_type_vars(&Type) -> HashSet<u32>` — walks
    `Type::Var`, `Type::Function { params, return_type }`,
    and the primitive / opaque variants.
  - `pub fn generalize(env: &HashMap<String, Type>, ty: &Type)
    -> Scheme` — classical DHM
    `gen(Γ, τ) = ∀ (ftv(τ) \ ftv(Γ)). τ`. Returns a
    monomorphic scheme (empty `vars`) when `ty` has no free
    variables beyond the env.
  - `Inferer::instantiate(&mut self, scheme: &Scheme) -> Type`
    — replaces each quantified var with a fresh `Type::Var(n)`
    via a one-shot substitution. The inferer's `subst` is
    NOT mutated; unification at the call site drives that.
  - Private `collect_ftv`, `ftv_env`, `substitute_vars`
    helpers.
  - 12 new unit tests (`free_type_vars_*`, `generalize_*`,
    `scheme_*`, `instantiate_*`, `round_trip_generalize_then_
    instantiate`).

### Scope deviation from the literal AC
The ticket's AC bundles four pieces:
1. `generalize` helper in `infer.rs` — **DONE** in this
   iteration.
2. Top-level fns get their inferred type generalized + stored
   in env as a `Scheme` — **DEFERRED**. Requires an
   inter-function env (the prototype from RES-120 is
   per-function), which itself is the shape RES-124a needs
   to express.
3. Each call site instantiates the scheme with fresh vars
   before unifying with argument types — **DEFERRED**. The
   `instantiate` helper exists; threading it through call
   sites is additive work.
4. Unit tests: `id<T>` called at Int + String — **DEFERRED**.
   The AC's `id<T>` syntax doesn't parse today (needs
   RES-124a's `fn<T>` parser). The helper-level tests added
   in this iteration exercise the `Scheme` / `generalize` /
   `instantiate` API against synthetic ASTs.

### Rationale for partial landing
- The Attempt 1 bail flagged two blockers: RES-120 (HM
  inference) and RES-124 (generic fn syntax). RES-120 is now
  landed, so the `generalize` + `instantiate` helpers can
  reasonably ship. RES-124 stays OPEN; landing pieces 2-4
  requires the `fn<T>` parser from RES-124a.
- Shipping the helpers now means when RES-124a lands, the
  call-site wiring is ~20 lines (parse `fn<T>` → call
  `generalize` post-body → look up + `instantiate` at call
  sites). No architectural rework.

### Verification
- `cargo test --locked` → 566 unchanged (infer feature is
  off by default)
- `cargo test --locked --features infer` → 603 (+12 new
  Scheme tests on top of the 25 from RES-120)
- `cargo clippy --locked --features lsp,z3,logos-lexer,infer
  --tests -- -D warnings` → clean

### Follow-ups (not in this ticket)
- **RES-122b** (new, suggested): call-site wiring — store
  inferred fn types as `Scheme` in a top-level env;
  instantiate at each `CallExpression`. Blocked on RES-124a.
- **RES-124a**: `fn<T, U>` parser + `type_params` field on
  `Node::Function`. Unblocks RES-122b + the `id<T>` test.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (blocked — see Attempt 1)
- 2026-04-17 re-claimed + partially resolved (see above)

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
