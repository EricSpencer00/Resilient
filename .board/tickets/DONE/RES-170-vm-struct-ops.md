---
id: RES-170
title: VM: struct literal + field load/store opcodes
state: DONE
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
- 2026-04-17 claimed by executor — landing RES-170a scope (struct registry only)
- 2026-04-17 landed RES-170a (struct registry); RES-170b/c/d deferred

## Resolution (RES-170a — struct registry only)

This landing covers only the **RES-170a** piece of the Attempt-1
clarification split: a struct registry keyed by decl name that
maps each `Node::StructDecl` to a `(type_id: u16, fields:
Vec<String>)` entry for RES-170c's lowering to consume. Compile-
time type propagation (170b), the three new opcodes + dispatch
(170c), and cross-module type_id uniqueness (170d) remain
deferred.

### Files changed

- `resilient/src/compiler.rs`
  - New `pub struct StructRegistryEntry { name, type_id, fields }`
    with `field_index(&str) -> Option<u8>` linear-scan lookup.
  - New `pub struct StructRegistry { entries: HashMap<String, Entry> }`
    with:
      * `StructRegistry::from_program(&Node) -> Result<Self, CompileError>`
        — walks `Program` top-level `Node::StructDecl`s, assigns
        each a unique `u16` type_id in source order, preserves
        field declaration order verbatim.
      * `len()` / `is_empty()` / `get(name)` / `resolve(struct_name,
        field_name) -> Option<(u16, u8)>` — the latter is the
        one-shot lookup RES-170c will call from its field-access
        lowering.
  - Inline block comment explains why this is compiler-local and
    not a shared module with RES-165a's JIT layout (different
    data — heap-backed `Vec<Value>` here vs. stack-resident
    repr(C) layout there; only the name→index map overlaps).
- `resilient/src/bytecode.rs`
  - Three new `CompileError` variants:
      * `DuplicateStructName(String)` — two decls collide.
      * `TooManyStructDecls` — more than u16::MAX + 1 registrations.
      * `TooManyFields(String)` — a single decl has more than
        u8::MAX + 1 fields (RES-170c's `LoadField { idx: u8 }` cap).
  - Matching `Display` arms.

### Tests (11 new, all `res170a_*`)

- Empty program → empty registry (`is_empty()` + `len() == 0`).
- Single `Point { int x, int y }` registers with `type_id == 0`.
- Field order preserved verbatim (not alphabetized): `c, a, b` →
  indices `0, 1, 2` via `field_index`.
- Missing field lookup returns `None`.
- Multiple struct decls get sequential `type_id`s (0, 1, 2) in
  source order.
- Duplicate struct name → `DuplicateStructName(name)` error.
- Unknown struct lookup returns `None`.
- `resolve()` roundtrips for struct decls at varying indices +
  varying field positions, including `None` for unknown names.
- Coexistence with `let` / `fn` decls: the registry ignores
  non-struct statements.
- Empty struct (`struct Empty { }`) registers with an empty field
  vector and `field_index("anything") == None`.
- Non-Program root errors with `Unsupported(_)`.

### Verification

```
$ cargo build                                   # OK (8 warnings, baseline)
$ cargo build --features z3                     # OK
$ cargo build --features lsp,logos-lexer,infer  # OK
$ cargo build --features jit                    # OK
$ cargo test --locked
test result: ok. 635 passed; 0 failed      (+11 vs 624)
$ cargo test res170a
test result: ok. 11 passed; 0 failed
```

### What was intentionally NOT done

- **RES-170b** — no compile-time type propagation pass
  (`local_slot → struct_name`). The registry is ready to be
  consumed; nothing consumes it yet.
- **RES-170c** — no `Op::MakeStruct` / `Op::LoadField` /
  `Op::StoreField` opcodes, no VM dispatch, no
  `Node::StructLiteral` / `Node::FieldAccess` /
  `Node::FieldAssignment` lowering arms.
- **RES-170d** — no cross-module type_id uniqueness after
  RES-073 import splicing.
- No changes to the existing VM semantics or interpreter paths.

### Follow-ups the Manager should mint

- **RES-170b** — compile-time type propagation threading a
  `local_slot → struct_name` map through `compile_expr` so
  `Node::FieldAccess { target: Identifier(p), field }` can be
  resolved to `(type_id, field_index)` via
  `StructRegistry::resolve`.
- **RES-170c** — three new opcodes + VM dispatch + compile-arms
  that consume RES-170b's map.
- **RES-170d** — cross-module type_id uniqueness across imports.

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
