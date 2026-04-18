---
id: RES-165
title: JIT: struct literal + field load/store (RES-072 Phase L)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Make structs — the most-used composite type after arrays — a
first-class citizen in the JIT. Phase L covers literal
construction (`Point { x: 1, y: 2 }`), field load (`p.x`), and
field store (`p.x = 3`, pending RES-153). Layout is a compact,
repr(C) struct with the field order from the declaration.

## Acceptance criteria
- For each `struct_decl` in the program, cache a layout: field
  name → (offset, cranelift type).
- Struct literals emit a stack slot (`ss = bcx.create_sized_stack_slot(...)`)
  and a sequence of `bcx.ins().stack_store(val, ss, offset)` calls.
- Field load: `bcx.ins().stack_load(field_ty, ss, offset)` when the
  struct lives in a stack slot. Through a pointer arg: `load(ty, flags,
  ptr + offset)`.
- Field store mirrors load.
- Struct-valued function returns: first cut is "return via out-ptr
  argument" ABI. Document the calling convention inline.
- Unit tests: literal + load, literal + store + load, struct as
  param, struct as return.
- Commit message: `RES-165: JIT struct literal + fields (Phase L)`.

## Notes
- Don't try to pass structs in registers yet — out-ptr works for
  every platform we target and keeps the lowering simple.
- Padding: align each field to its natural alignment. Document the
  layout algorithm inline so it matches the interpreter/VM view
  when serialized.

## Log
- 2026-04-17 created by manager
