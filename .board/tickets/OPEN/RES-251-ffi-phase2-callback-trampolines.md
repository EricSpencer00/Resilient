---
id: RES-251
title: "FFI Phase 2: callback function-pointer trampolines in bytecode VM"
state: OPEN
priority: P3
goalpost: G14
created: 2026-04-20
owner: executor
---

## Summary

FFI Phase 1 (RES-094 area) can call C functions from Resilient and pass scalar
arguments. However, passing a Resilient function as a callback to a foreign
function is explicitly rejected with the message:

> "FFI: extern fn `{}` uses a Callback parameter; callbacks require the
> trampoline feature (planned for Phase 2)"

Phase 2 should build the trampoline infrastructure so Resilient functions can
be passed as `extern "C"` function pointers to foreign code. The interpreter
path is the first priority; JIT support can follow in a separate ticket.

## Acceptance criteria

- The `ffi.rs` module no longer returns `FfiError` for `Callback`-typed
  parameters; instead it manufactures a C-callable function pointer.
- The trampoline bridges the C calling convention to the Resilient VM:
  - Receives C-ABI arguments, marshals them to `Value` instances, calls the
    Resilient function through the interpreter, and returns the result.
- The `ffi_trampolines.rs` test
  `call_foreign_rejects_callback_argument_as_phase_1_stub` is updated (or
  replaced) to verify the happy path works.
- No `unsafe` is introduced without a justifying comment explaining the
  invariant.
- No new `std`-only types in the `resilient-runtime` path.
- Commit message: `RES-251: FFI Phase 2 callback trampolines (interpreter path)`.

## Notes

- `resilient/src/ffi.rs` line 129–171 is the relevant stub.
- `resilient/src/ffi_trampolines.rs` line 378–397 has the existing Phase 1
  rejection test.
- The `libffi` or `frunk`-style approach may be needed for portable C-ABI
  function pointer creation. Evaluate `libffi` crate vs. hand-rolled assembly
  for the embedded targets.
- JIT path (passing a JIT-compiled closure as callback) is a separate follow-up.

## Log
- 2026-04-20 created by analyzer (stub in ffi.rs, line ~171, with "Phase 2" note)
