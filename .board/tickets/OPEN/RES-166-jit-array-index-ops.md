---
id: RES-166
title: JIT: array indexed load/store (RES-072 Phase M)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Arrays aren't native to Cranelift either. Give the JIT an
`Array<Int>` lowering using a heap-allocated buffer with a small
runtime shim for bounds-checks. Once this lands plus RES-165, most
of our example programs can run end-to-end through the JIT.

## Acceptance criteria
- Runtime-side shim functions exposed as FFI symbols:
  `res_array_new(len: i64) -> *mut Array`,
  `res_array_get(arr: *mut Array, i: i64) -> i64`,
  `res_array_set(arr: *mut Array, i: i64, v: i64)`.
- Cranelift: emit `call_indirect` through an absolute-address
  constant set at JIT-init time (match how we already wire the
  existing runtime functions).
- Bounds checks: the shim panics with a clean error; JIT does not
  need to check inline (simpler first cut).
- If RES-131 elides bounds, the JIT still calls the checking
  shim — no correctness issue, only a small perf loss. Perf ticket
  tracks unchecked variants.
- Unit tests + smoke test: `[1,2,3][1]` → 2; OOB → clean runtime
  error; array-sum-loop benchmark in `benchmarks/jit/` runs and
  matches the tree-walker output.
- Commit message: `RES-166: JIT array indexed load/store (Phase M)`.

## Notes
- Reusing the existing `Vec<Value>`-based array from the
  interpreter through FFI makes integers cheap (i64) but anything
  else passes through `Value` and is slow. OK for now — most
  hot-loop benchmarks use int-only arrays.
- Keep the shim in `resilient/src/jit_backend.rs` under a
  `mod runtime_shims` submodule so it's findable.

## Log
- 2026-04-17 created by manager
