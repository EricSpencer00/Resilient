---
id: RES-171
title: VM: array literal + index load/store + push/pop opcodes
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Arrays are the last big value-kind the VM doesn't handle. With
RES-170 + this, the VM will cover all example programs that the
interpreter runs.

## Acceptance criteria
- Opcodes:
  - `MakeArray { len: u16 }` — pops len values, pushes Array.
  - `LoadIndex` — pops idx + arr, pushes arr[idx]. Bounds check
    inline with clean runtime error using per-statement line info
    (RES-092).
  - `StoreIndex` — pops v, idx, arr; writes back.
  - `ArrayPush` / `ArrayPop` / `ArraySlice` — call into runtime
    helper functions (same approach as interpreter / JIT shims).
- Compiler lowers `[a, b, c]` → 3 evals + MakeArray 3.
- `a[i] = v;` lowers to StoreIndex; support for nested
  (`a[i][j] = v`) produced by sequential LoadIndex + StoreIndex
  that matches RES-034 semantics.
- Unit tests: literal round-trip, indexing, push/pop/slice,
  nested assignment.
- Commit message: `RES-171: VM array ops`.

## Notes
- Watch the existing `bytecode.rs` opcode enum — keep the variant
  width reasonable. If the enum gets too wide, consider a
  `Op::ArrayOp(ArrayKind)` subvariant.
- Performance: the VM's array ops allocate and deallocate heap
  memory on every array manipulation. Acceptable; peephole pass
  (RES-172) can coalesce some patterns.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (oversized; 6 opcodes +
  runtime helpers + compiler lowering + tests)

## Attempt 1 failed

Oversized: the ticket bundles four independently-sized pieces.

1. **Six new opcodes** (`MakeArray`, `LoadIndex`, `StoreIndex`,
   `ArrayPush`, `ArrayPop`, `ArraySlice`) in `src/bytecode.rs` +
   VM dispatch arms in `src/vm.rs`.
2. **Runtime helper functions** for push / pop / slice exposed to
   the VM — same scaffolding concept RES-166 introduces for the
   JIT's `mod runtime_shims`, which also doesn't exist yet.
3. **Compiler lowering** for `Node::ArrayLiteral`,
   `IndexExpression`, `IndexAssignment` (including the nested
   `a[i][j] = v` form). Today the VM compiler errors
   `Unsupported` on all of these.
4. **Per-op bounds-check error paths** carrying `line_info`
   (RES-092), plus the ticket's four end-to-end tests.

## Clarification needed

Manager, please split:

- RES-171a: `MakeArray` + `LoadIndex` + `StoreIndex` opcodes +
  dispatch + bounds-check error path. Compile `ArrayLiteral` +
  simple `IndexExpression` / `IndexAssignment`. Smallest self-
  contained slice.
- RES-171b: `ArrayPush` / `ArrayPop` / `ArraySlice` via runtime
  helpers. Consider hoisting the VM's runtime-shim scaffolding
  into its own shared ticket if the JIT side (RES-166) also wants
  it — both tickets propose parallel mod-level shims.
- RES-171c: nested `a[i][j] = v` lowering matching RES-034
  semantics.

No code changes landed — only the ticket state toggle and this
clarification note.
