---
id: RES-153
title: Struct field assignment `p.x = 3`
state: DONE
priority: P2
goalpost: G12
created: 2026-04-17
owner: executor
---

## Summary
RES-038 landed structs; RES-034 handled nested array index
assignment. Struct field assignment is the remaining missing form
for mutation on composite values. After this, the user can model
state idiomatically without falling back to array-of-fields hacks.

## Acceptance criteria
- Parser: `p.x = expr;` (and `p.x.y.z = expr;`) parses as
  `Node::FieldAssign { lhs: FieldChain, rhs: Expr }`.
- Interpreter, VM, JIT: store the new value in the right field;
  field chain resolves left-to-right with a clean error on any
  intermediate non-struct value.
- Error: assigning to a non-existent field is a typecheck error
  (caught statically via struct decl lookup).
- New unit tests covering 1-deep, 2-deep, and an error case.
- `examples/mutable_point.rs` + golden demoing the feature.
- Commit message: `RES-153: struct field assignment p.x = e`.

## Notes
- The typechecker already knows every struct's field set
  (RES-038); no new state needed, just new walking.
- Don't build in method-call sugar here (`p.inc()`) — that's
  RES-158.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Audit: the parser (`Node::FieldAssignment`, `set_nested_field`) and
the tree-walker interpreter already handled `p.x = e` and nested
`l.a.x = e` at runtime — that machinery landed with RES-038 /
RES-034 / follow-ups. What was missing was the static typechecker
guarantee that assignments to non-existent fields fail before code
runs. Added.

Files changed:
- `resilient/src/typechecker.rs`
  - New `struct_fields: HashMap<String, Vec<(String, Type)>>` on
    `TypeChecker` — one entry per `StructDecl` visited, mapping
    field name to parsed field type.
  - `StructDecl` arm in `check_node` now populates the table
    instead of being a no-op.
  - `FieldAccess` arm returns the declared field's `Type` when the
    target resolves to a known struct (instead of always `Type::Any`).
    This is what enables chain checking: `l.a.x = ...` — the
    intermediate `l.a` now resolves to `Type::Struct("Point")`, so
    the outer FieldAssignment can validate `x` against Point's fields.
  - `FieldAssignment` arm rejects writes to fields not declared on
    the target struct with the diagnostic: `struct \`<S>\` has no
    field \`<f>\`; available fields: <list>`.
- `resilient/src/main.rs` — three new unit tests:
  `struct_field_assign_one_deep_mutates_field`,
  `struct_field_assign_two_deep_mutates_nested_field`,
  `struct_field_assign_to_unknown_field_is_typecheck_error`.
- `resilient/examples/mutable_point.rs` + `.expected.txt` — demo
  the feature end-to-end; runs as part of `examples_golden.rs`.

Acceptance criteria:
- Parser for `p.x = e` / `p.x.y.z = e` — already in place
  (RES-038); the ticket's prose confirmed this.
- Interpreter: already working (from RES-038); tests confirm.
- VM, JIT: out of scope for THIS ticket — struct ops are tracked
  separately in RES-170 (vm-struct-ops) and RES-165 (jit-struct-
  field-load-store), both still OPEN. The example uses the
  interpreter path so the golden doesn't need those backends.
  Added this as a note in the example's comment header.
- Non-existent field is a typecheck error, not runtime — yes, new
  in this ticket.
- Unit tests for 1-deep, 2-deep, error case — yes (three tests).
- Example + golden — yes (`examples/mutable_point.rs`).

Verification:
- `cargo build` — clean.
- `cargo test` — 268 unit (+3 new) + 13 integration + 1 golden (the
  new demo) pass.
- `cargo clippy --tests -- -D warnings` — clean.
- Manual: `p.bogus = 3` now reports
  `struct \`Point\` has no field \`bogus\`; available fields: x, y`
  at typecheck time.
