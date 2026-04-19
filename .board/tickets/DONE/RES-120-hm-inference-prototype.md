---
id: RES-120
title: Hindley-Milner inference prototype over Int / Bool / Float / String
state: DONE
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
- 2026-04-17 re-claimed by executor — Attempt 1 blocker 1
  (Diagnostic) cleared by RES-119 scaffolding; blocker 2
  (NodeId) worked around via Option 2 substitution-map shape.
- 2026-04-17 resolved by executor (primitive-only Algorithm W
  + 25 unit tests; gated on `infer` feature)

## Resolution

### Files added
- `resilient/src/infer.rs` — new opt-in module. Public API:
  - `pub struct Inferer { next_var, subst, env }`.
  - `Inferer::new()`, `Inferer::fresh()`, `Inferer::substitution()`.
  - `Inferer::infer_function(&Node) -> Result<Substitution,
    Vec<Diagnostic>>`. Seeds the env from declared parameter
    types, walks the body, solves constraints via RES-121's
    unify module, returns the final substitution.
  - Six stable error codes: `T0001_OCCURS`,
    `T0002_PRIMITIVE_MISMATCH`, `T0003_STRUCTURED_MISMATCH`,
    `T0004_ARITY_MISMATCH`, `T0005_UNBOUND`, `T0006_UNSUPPORTED`.
- 25 unit tests in `infer::tests` covering every AC bullet:
  literal inference (Int/Float/Bool/String), parameter env
  seeding, every hard-coded operator rule (arithmetic,
  logical, comparison, bitwise, prefix `!`, prefix `-`), let
  annotation match + conflict, if-condition must-be-bool,
  while-condition must-be-bool, unbound-identifier, occurs-
  check error-code mapping, substitution accessor.

### Files changed
- `resilient/Cargo.toml` — new `infer = []` feature. Opt-in
  per the ticket: "feature-gated behind `infer`; RES-123 flips
  it on by default once the surface is covered."
- `resilient/src/main.rs` — `#[cfg(feature = "infer")] mod infer;`.

### Scope deviation from the literal AC (documented in module
header)
- **Return type** is `Result<Substitution, Vec<Diagnostic>>`
  instead of `Result<HashMap<NodeId, Type>, Vec<Diagnostic>>`.
  The bail's Option 2 recommended this: `NodeId` doesn't
  exist in the codebase (no stable id-per-node infra), but a
  substitution map keyed by type-var id is sufficient to
  surface the inference result. RES-123 (or a dedicated
  NodeId ticket) can migrate to the NodeId-keyed shape later.

### Design decisions
- **Accumulating diagnostics.** `infer_stmt` pushes to a
  `&mut Vec<Diagnostic>` rather than short-circuiting on the
  first error. Matches rustc's "emit everything we can so the
  user sees the full picture" policy. Downstream consumers
  get the full error set from one pass.
- **Single-error return from `infer_expr`.** Expression-level
  failures propagate as a single `Err(Diagnostic)` — the
  caller decides whether to accumulate or bail. `infer_stmt`
  accumulates; future call-site migrations (RES-119c) can
  choose differently.
- **Fresh var on expression-level unsupported shape.**
  Returning a fresh var means unification at the use-site
  pins the variable, so downstream error messages are more
  specific. The alternative (`T0006 unsupported`) would
  explode the error count on any partial AST.
- **`Type::Var` chained through RES-121's `apply`.** The
  `substitution()` accessor returns the raw map; consumers
  call `subst.apply(ty)` to get concrete types. Matches the
  unify module's idiomatic use.

### Also-unblocks
- **RES-122** (let-polymorphism) — can now build on
  `infer::Inferer` + call generalize at binding time.
- **RES-124** (generic `fn<T>`) — the occurs-check is
  already present in RES-121; generics need the
  instantiation-at-call-site walk on top.
- **RES-125** (type holes `_`) — the inferer's `fresh()`
  method is the target: a `_` in a type position becomes a
  fresh var tagged with the `_`-origin span.

### Verification
- `cargo test --locked` → 566 passed (unchanged — `infer`
  is off by default)
- `cargo test --locked --features infer` → 591 passed
  (+25 new infer tests)
- `cargo clippy --locked --features lsp,z3,logos-lexer,infer
  --tests -- -D warnings` → clean
- `cargo clippy --locked --features lsp,z3,logos-lexer
  --tests -- -D warnings` → clean (no regression without the
  feature)

### Follow-ups (not in this ticket)
- **RES-122**: let-polymorphism — wrap inferred types in a
  `Scheme`, generalize at fn-binding time.
- **RES-124**: generic `fn<T>` syntax + instantiation.
- **RES-125**: `_` type holes.
- **Array/Struct/Result inference.** The prototype returns a
  fresh var for unsupported AST shapes; full coverage needs
  type-constructor threading through unify (RES-127 / RES-055
  territory).
- **Function-call inference.** The prototype walks expressions
  but doesn't look up user-fn signatures (each call returns a
  fresh var). A follow-up could build a global fn-env before
  the body walk.
- **RES-123**: flip `infer` to default-on once coverage is
  wide enough to not break any existing programs.

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
