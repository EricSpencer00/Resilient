---
id: RES-121
title: Constraint-based unification with occurs check, split from infer prototype
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/unify.rs` (new): `Substitution`, `unify`, `apply`,
  `compose`, `bind`, `occurs`, plus `UnifyError` with `Mismatch`,
  `Occurs`, and `ArityMismatch` variants. Idempotent `apply`,
  occurs-check run before every `Var` binding, and "apply-other-
  first-then-self" composition semantics documented on `compose`.
- `resilient/src/typechecker.rs`: added `Type::Var(u32)` variant
  (with dead-code allow pending RES-120) and wired its `Display`
  form as `?tN`.
- `resilient/src/main.rs`: declared `mod unify;` unconditionally so
  the inference walker (RES-120) can pick it up once rewritten.

Coverage of the ticket's test matrix (all in `mod tests` at the
bottom of `unify.rs`, 15 tests total):
- prim ↔ prim success (`prim_unifies_with_itself`)
- prim ↔ prim failure (`prim_mismatch_errors_with_the_two_types`)
- var unified to prim (`var_unifies_to_prim`)
- var unified to var then to prim (`var_unifies_to_var_and_then_to_prim`)
- var-to-self noop (`var_equal_to_itself_is_noop`)
- occurs check direct (`occurs_check_catches_direct_self_reference`)
- occurs check indirect via chain
  (`occurs_check_catches_indirect_self_reference_via_chain`)
- idempotent apply on three-variable chain
  (`apply_is_idempotent_on_three_variable_chain`) — the test the
  ticket's notes section explicitly calls out
- apply into function types (`apply_recurses_into_function_types`)
- elementwise function unification (`unify_function_types_elementwise`)
- arity mismatch (`unify_function_arity_mismatch_errors`)
- compose ordering (`compose_apply_other_first_then_self`)
- compose preserves self bindings
  (`compose_preserves_self_bindings_for_unrelated_vars`)
- `Any` back-compat (`any_accepts_anything_for_back_compat_with_res053`)
- struct name mismatch (`struct_mismatch_errors`)

Deviation from the ticket: the acceptance criterion "infer.rs
refactored to use the new module; no behavior change from RES-120"
cannot land yet — RES-120 is currently in OPEN with a
`## Clarification needed` note (blocked on RES-119's Diagnostic
scaffolding and an absent NodeId). The refactor hook is in place:
`mod unify;` is unconditional and `Type::Var` is live, so when
RES-120 gets rewritten and lands, the walker can import the module
directly. Calling this ticket done for its independently-verifiable
scope; the Manager can open a follow-up if the RES-120 integration
check wants its own pass.

Verification:
- `cargo build` — clean.
- `cargo test` — 248 unit (+15 new) + 13 integration pass.
- `cargo clippy --tests -- -D warnings` — clean.
- `cargo clippy --features logos-lexer --tests -- -D warnings` —
  clean.
