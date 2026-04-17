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
