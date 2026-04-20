---
id: RES-250
title: "RES-165d: JIT struct-valued function returns via out-ptr ABI"
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-20
owner: executor
---

## Summary

The final deferred sub-task of RES-165. A JIT-compiled function whose return
type is a struct must hand the value back to the caller via an out-pointer
argument (the agreed calling convention for all embedded targets). Currently
struct-valued returns are unsupported in the JIT.

## Acceptance criteria

- Functions returning a struct type receive a hidden first argument: a pointer
  to a caller-allocated buffer sized for the struct.
- On return, the struct fields are written into that buffer via `store`
  instructions using the `StructLayout` offsets.
- The caller allocates a suitably-sized buffer, passes its address, then reads
  fields from the buffer after the call returns.
- Unit test: a function returning `Point { x: a, y: b }` called from a JIT
  harness produces correct field values in the caller's buffer.
- The calling convention is documented inline (comment block in `jit_backend.rs`).
- No regressions in existing JIT tests.
- Gated on RES-248 and RES-249 landing first.
- Commit message: `RES-250: JIT struct-valued return via out-ptr ABI (RES-165d)`.

## Notes

- Original design intent documented in `jit_backend.rs` around line 1763–1778.
- The out-ptr ABI avoids register-passing complexity and works uniformly on
  `thumbv7em-none-eabihf`, `thumbv6m-none-eabi`, and `riscv32imac-unknown-none-elf`.

## Log
- 2026-04-20 created by analyzer (deferred sub-task of DONE/RES-165)
