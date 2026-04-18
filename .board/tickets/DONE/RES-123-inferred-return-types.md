---
id: RES-123
title: Drop mandatory explicit return types; infer when omitted
state: DONE
priority: P3
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
Today you have to write `fn square(Int x) -> Int { return x * x; }`.
Once HM inference lands (RES-120..122), the `-> Int` is redundant.
Make the return type annotation optional and have the inference
engine fill it in.

## Acceptance criteria
- Parser: `fn name(params) { ... }` parses with no return type;
  the AST holds `Option<TypeAnnotation>`.
- Typechecker / inferer: if the annotation is present, unify the
  inferred return type against it (existing behavior).
  If absent, leave the inferred type as-is and store it on the
  function node for later phases.
- `fn name() { ... }` with no explicit ret type and no `return`
  stmt infers `Type::Void`.
- Unit tests covering: omitted annotation succeeds, omitted
  annotation disagrees with body (should never trigger, but the
  test pins that inference gives a sensible result), explicit
  annotation still overrides (unify fails if wrong).
- SYNTAX.md updated to show both forms (explicit + inferred).
- Commit message: `RES-123: optional return type annotations`.

## Notes
- Don't loosen parameter types — those stay required. Reason: at a
  function boundary, inferring parameter types from call-site
  usage is a worse developer experience (errors fire at callers,
  not at the definition). Return types are safe to infer because
  the body already defines them.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Audit finding: the mechanics this ticket asks for already shipped
in the existing codebase — RES-052 landed `return_type:
Option<String>` on `Node::Function` plus
`parse_optional_return_type`, and RES-053 added the typechecker's
"if declared: unify against body; else use body type" logic. A
function without `-> TYPE` already typechecks and runs. The
ticket's scope therefore reduces to:

- **Tests** pinning that behavior so the next refactor can't
  silently regress it.
- **SYNTAX.md** documenting the optional form for users.

Files changed:
- `resilient/src/main.rs` — three new unit tests in `mod tests`:
  - `fn_without_return_type_annotation_typechecks`:
    `fn square(int x) { return x * x; }` typechecks.
  - `fn_with_explicit_return_type_still_checks_against_body`:
    `fn square(int x) -> bool { return x * x; }` fails with
    `return type mismatch — declared bool, body produces int`
    (the RES-053 diagnostic is preserved).
  - `fn_with_no_return_stmt_infers_void`: both
    `fn sink(...) -> void {...}` and `fn sink(...) {...}` with
    only a side-effecting body typecheck identically.
- `SYNTAX.md` — new `### Return types` subsection under
  `## Function Declarations` showing both forms, noting the
  explicit-disagreement diagnostic, and calling out why
  parameter types stay required (per the ticket's notes).

Deferred: deep inference on expression-level bodies — blocks on
RES-120 (HM inference prototype, currently OPEN with
Clarification needed). The RES-053 "body_type" fallback is the
limited inferer that makes today's surface work; RES-120 will
upgrade it to full Algorithm W when it lands.

Verification:
- `cargo build --locked` — clean.
- `cargo test --locked` — 285 unit (+3 new) + all integration
  pass.
- `cargo clippy --locked --tests -- -D warnings` — clean.
- Manual: `fn square(int x) { return x * x; }` + `println(square
  (7));` typechecks and prints `49`.
