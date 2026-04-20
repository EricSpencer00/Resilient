# RES-NEW: FFI Phase 1 — tree-walker + static registry

## Summary
Ships primitive-only FFI for the tree-walker interpreter on std hosts
and the static-registry path for no_std embedded.

## Acceptance
- [x] `extern "lib" { fn ... }` blocks parse
- [x] Typechecker rejects non-primitive FFI types and @pure on extern
- [x] Loader resolves symbols on std via libloading (ffi feature, opt-in)
- [x] Tree-walker dispatches through the trampoline table
- [x] `requires` and `ensures` checked at runtime on FFI calls
- [x] `@trusted` propagates ensures as SMT assumption (not a runtime abort)
- [x] `resilient-runtime` ffi-static registry (no_std, zero-alloc)
- [x] End-to-end C helper library + integration tests (Linux + macOS)
- [x] Example program (ffi_libm.rs) + SYNTAX.md FFI section + docs/ffi.md

## Out of scope (filed as follow-ups)
- Bytecode VM `OP_CALL_FOREIGN` (phase 2)
- Cranelift JIT lowering (phase 3)
- Struct / Array / callback marshalling
- Variadic foreign fns
- Wire trusted extern ensures into typechecker Z3 path (typechecker.rs)
