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
- 2026-04-17 claimed and bailed by executor (needs compile-time type info)

## Attempt 1 failed

The hard requirement "Field names resolve to indices at compile
time; runtime never sees the string name" forces compile-time type
resolution, and the VM compiler (`src/compiler.rs`) has none —
it emits `Unsupported` on `FieldAccess` / `FieldAssignment` /
`StructLiteral` today precisely because it can't answer "what
struct type is `p`?" without walking inference.

Breakdown of the bundled work:

1. Struct registry (decl name → `type_id` + `Vec<field_name>` in
   declaration order). Shareable with RES-165 (JIT struct ops).
2. Compile-time type tracker — thread `local_slot -> struct_name`
   through `compile_expr` so `Node::FieldAccess { target:
   Identifier(p), field }` resolves to `(type_id, field_idx)`.
   This is essentially rebuilding struct-aware type propagation in
   a second pass.
3. Three new opcodes (`MakeStruct`, `LoadField`, `StoreField`) +
   VM dispatch.
4. Cross-module `type_id` uniqueness across imports.

Each of 1–3 is iteration-sized; 2 is the real cost.

## Clarification needed

Manager, please split:

- RES-170a: struct registry (decl → type_id + field-name vec),
  shareable with RES-165.
- RES-170b: compile-time type propagation for locals that hold
  structs — let-bindings from struct literals, fn params typed as
  struct, fn returns declared as struct. Output: per-function
  `local_slot -> struct_name` map.
- RES-170c: the three opcodes + VM dispatch + compile-arms that
  consume 170b's map.
- RES-170d: cross-module `type_id` uniqueness across imports.

No code changes landed — only the ticket state toggle and this
clarification note.
