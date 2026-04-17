---
id: RES-153
title: Struct field assignment `p.x = 3`
state: OPEN
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
