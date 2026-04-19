---
id: RES-165
title: JIT: struct literal + field load/store (RES-072 Phase L)
state: DONE
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
- 2026-04-17 claimed by executor — landing RES-165a scope (layout cache only)
- 2026-04-17 landed RES-165a (layout cache); RES-165b/c/d deferred

## Resolution (RES-165a — layout cache only)

This landing covers only the **RES-165a** piece of the Attempt-1
clarification split: a struct layout cache keyed by decl name.
Literal construction (RES-165b), field load/store (RES-165c), and
struct-valued returns via out-ptr (RES-165d) remain deferred.

### Files changed

- `resilient/src/jit_backend.rs`
  - New types `FieldLayout` (name / offset / cranelift `Type` /
    size / align) and `StructLayout` (name / fields in decl order
    / total size / alignment).
  - `StructLayout::field(name)` — O(N) lookup by field name.
  - Helper `cranelift_ty_for(annotation)` maps surface type names
    to `(Type, size, align)` triples:
      `int` / `Int` / `I64`   → I64 (8/8)
      `float` / `Float` / `F64` → F64 (8/8)
      `i32` / `I32`           → I32 (4/4)
      `bool` / `Bool`         → I8  (1/1)
      (fallback)              → I64 pointer (8/8)
  - `build_struct_layout(&Node)` implements classic repr(C)
    placement: fields in decl order, each aligned-up to its
    natural alignment, struct total size rounded up to its own
    alignment so arrays-of-struct tile correctly.
  - `pub(crate) fn collect_struct_layouts(program) -> HashMap<String, StructLayout>`
    walks a `Program` and returns the full cache. Ignores
    non-`StructDecl` statements so it's safe to call over a
    realistic program that also has lets / fns / etc.
  - Inline block-comment spec explains the layout algorithm and
    the type mapping so RES-165b/c/d can lower without guessing.
- Thirteen new unit tests named `res165a_*` in the jit tests
  module cover:
  - Empty program → empty cache.
  - Two `int` fields at offsets 0, 8 (size 16, align 8).
  - `bool` then `int` — 7 bytes of padding, b@0 x@8, size 16.
  - `int` then trailing `bool` — trailing pad to align 8.
  - Three `bool`s pack tightly (size 3, align 1).
  - `float` + `int` both 8-aligned, F64 used for float.
  - Two `i32`s pack at 4-byte alignment, total size 8.
  - Empty struct → size 0, align 1.
  - Multiple struct decls all cached.
  - Unknown struct name lookup → `None`.
  - `StructLayout::field()` roundtrip for each declared field.
  - Unknown field type falls back to pointer (I64).
  - Layouts collected correctly when intermixed with let/fn decls.

### Verification

```
$ cargo build                                   # OK (8 warnings, baseline)
$ cargo build --features jit                    # OK
$ cargo test --locked
test result: ok. 611 passed; 0 failed        (non-jit baseline)
$ cargo test --locked --features jit
test result: ok. 697 passed; 0 failed        (+13 new)
$ cargo test --features jit res165a
test result: ok. 13 passed; 0 failed
```

### What was intentionally NOT done

- **RES-165b** — no struct literal lowering. `StructLiteral`
  nodes still produce `JitError::Unsupported`.
- **RES-165c** — no field load/store lowering.
- **RES-165d** — no struct-valued return via out-ptr ABI.
- No changes to the calling convention, no changes to any
  existing lowering path.
- Layouts are `pub(crate)` so RES-165b/c/d can consume them
  without exposing the surface to other modules.

### Follow-ups the Manager should mint

- **RES-165b** — literal construction via sized stack slot +
  `stack_store(val, ss, offset)` per field, consuming this cache.
- **RES-165c** — field load/store for stack-resident structs.
- **RES-165d** — out-ptr ABI for struct-valued returns + end-to-
  end test.

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
