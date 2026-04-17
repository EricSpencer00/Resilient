---
id: RES-170
title: VM: struct literal + field load/store opcodes
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Mirror of RES-165 but for the bytecode VM. The VM's struct story
uses a heap-allocated `Vec<Value>` indexed by field position
(resolved at compile time).

## Acceptance criteria
- Opcodes:
  - `MakeStruct { type_id: u16, field_count: u8 }` — pops N values
    off the stack, pushes a struct Value.
  - `LoadField { idx: u8 }` — pops struct, pushes field.
  - `StoreField { idx: u8 }` — pops value + struct, writes back.
- Compiler: struct-literal AST lowers the fields in declaration
  order, then emits MakeStruct.
- Field names resolve to indices at compile time; runtime never
  sees the string name.
- Unit tests: roundtrip literal → load, struct reassignment,
  struct passed into a function.
- `type_id` uniqueness across imports: use a post-import global
  registry in the compiler (after RES-073's splice).
- Commit message: `RES-170: VM struct ops`.

## Notes
- Structs-on-stack (unboxed) is a future optimization — pay the
  heap cost for now. Matches the interpreter's representation,
  which keeps mental model tidy.
- `type_id` is informational (for debug dumps); runtime
  semantics don't depend on it.

## Log
- 2026-04-17 created by manager
