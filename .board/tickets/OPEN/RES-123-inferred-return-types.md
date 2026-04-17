---
id: RES-123
title: Drop mandatory explicit return types; infer when omitted
state: OPEN
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
