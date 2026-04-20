---
id: RES-248
title: "RES-165b: JIT lower struct literals to Cranelift IR"
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-20
owner: executor
---

## Summary

RES-165 landed the struct layout cache (`RES-165a`) but deferred the actual
Cranelift IR lowering for struct literals (165b), field load/store (165c), and
struct-valued returns via out-ptr ABI (165d). This ticket covers **RES-165b**:
lowering `StructLiteral` nodes in `lower_expr` to a sized stack slot with a
sequence of `stack_store` calls.

Currently, `lower_expr` falls through to `Err(JitError::Unsupported(node_kind(node)))`
for any `StructLiteral` node. The layout cache built in `jit_backend.rs` is
`pub(crate)` and ready for consumption.

## Acceptance criteria

- `lower_expr` handles `Node::StructLiteral { name, fields, .. }`:
  - Look up the `StructLayout` for `name` in the cached layout map.
  - Allocate a sized stack slot: `bcx.create_sized_stack_slot(StackSlotData::new(kind, size, align))`.
  - For each field value, lower the expression to a `Value` and emit `bcx.ins().stack_store(val, ss, offset)`.
  - Return the stack slot as a `StackValue` (or pointer) for downstream use by 165c/d.
- Unit test: construct `Point { x: 1, y: 2 }` via JIT and verify the returned slot contains correct field values.
- No regressions in existing JIT tests.
- Commit message: `RES-248: JIT struct literal lowering (RES-165b)`.

## Notes

- See `jit_backend.rs` lines 1760–1778 for the design intent and the existing `StructLayout` / `FieldLayout` types.
- Coordinate with RES-249 (field load/store) and RES-250 (out-ptr return ABI) — they share the same stack slot representation.
- The `node_kind` fallthrough at line 1733 is the exact path currently hit by struct literals.

## Log
- 2026-04-20 created by analyzer (deferred sub-task of DONE/RES-165)
