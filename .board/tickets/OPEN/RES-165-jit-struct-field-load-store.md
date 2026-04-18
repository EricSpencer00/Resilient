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
- 2026-04-17 claimed and bailed by executor (oversized JIT lowering)

## Attempt 1 failed

Oversized: this ticket is new Cranelift IR lowering across four
distinct code paths (literal construction with stack slots, field
load, field store, struct-valued returns via an out-ptr ABI), plus
a layout cache keyed by struct decl, plus four end-to-end tests
driving `--features jit`.

The existing JIT backend (`src/jit_backend.rs`, 1137 lines) has no
struct lowering today — no stack-slot emission for composites, no
layout cache, no out-ptr ABI. Landing all four pieces in one
iteration on top of an unfamiliar Cranelift backend is not
realistic.

## Clarification needed

Manager, please split into:

- RES-165a: struct layout cache
  (`decl_name -> Vec<(field, offset, cranelift_ty)>`) built from
  every `Node::StructDecl` before lowering. Testable in isolation
  by asserting offsets for a known decl.
- RES-165b: literal construction — emit a sized stack slot and the
  sequence of `stack_store(val, ss, offset)` calls per field.
- RES-165c: field load + store when the struct lives in a stack
  slot (`p.x` read + `p.x = v` write).
- RES-165d: struct-valued return via out-ptr ABI, plus one end-to-
  end test that builds a struct in a compiled fn and returns it.

No code changes landed — only the ticket state toggle and this
clarification note.
