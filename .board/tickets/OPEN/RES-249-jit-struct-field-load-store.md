---
id: RES-249
title: "RES-165c: JIT lower struct field load and store to Cranelift IR"
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-20
owner: executor
---

## Summary

Companion to RES-248. After struct literals can be constructed in the JIT
(RES-165b / RES-248), field reads (`p.x`) and field writes (`p.x = 3`) must
also be lowered. Currently `FieldAccess` and `FieldAssignment` nodes fall
through to `Err(JitError::Unsupported(...))` in `lower_expr` and the
statement lowering pass.

## Acceptance criteria

- `lower_expr` handles `Node::FieldAccess { object, field, .. }`:
  - Lower `object` to a stack slot or pointer.
  - Emit `bcx.ins().stack_load(field_ty, ss, offset)` using the offset from
    `StructLayout::field_offset(field)`.
- The statement lowering pass handles `Node::FieldAssignment { object, field, value, .. }`:
  - Lower `value` to a `Value`.
  - Emit `bcx.ins().stack_store(val, ss, offset)`.
- Unit tests: field load after literal construction, field store then load.
- No regressions in existing JIT tests.
- Gated on RES-248 (struct literal lowering) landing first.
- Commit message: `RES-249: JIT struct field load/store (RES-165c)`.

## Notes

- `StructLayout` and `FieldLayout` are in `jit_backend.rs`; both are `pub(crate)`.
- The type mapping (int → `types::I64`, bool → `types::I8`, float → `types::F64`)
  is documented in the `jit_backend.rs` block comment around line 1796.

## Log
- 2026-04-20 created by analyzer (deferred sub-task of DONE/RES-165)
