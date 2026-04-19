---
id: RES-166
title: JIT: array indexed load/store (RES-072 Phase M)
state: DONE
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
- 2026-04-17 claimed and bailed by executor (oversized; first JIT FFI)
- 2026-04-17 claimed by executor — landing RES-166a scope (runtime shims + symbol wiring)
- 2026-04-17 landed RES-166a (shims + JITBuilder::symbol wiring); RES-166b/c/d deferred

## Resolution (RES-166a — runtime shims + symbol wiring)

This landing covers only the **RES-166a** piece of the Attempt-1
clarification split: the runtime shim module + `JITBuilder::symbol`
registrations. `ArrayLiteral` / `IndexExpression` lowering
(RES-166b), `IndexAssignment` lowering (RES-166c), and the
bench/smoke test (RES-166d) remain deferred.

### Files changed

- `resilient/src/jit_backend.rs`
  - New `pub(crate) mod runtime_shims` with four `extern "C-unwind"`
    shim fns:
      * `res_array_new(len: i64) -> *mut ResArray` — allocate a
        zero-initialized Vec<i64>, clamping negative lens to 0.
      * `res_array_get(arr, i) -> i64` — bounds-checked read,
        panics cleanly on null ptr or OOB index.
      * `res_array_set(arr, i, v)` — bounds-checked write, same
        panic contract.
      * `res_array_free(arr)` — reclaim the Box; null is a no-op.
  - `ResArray { items: Vec<i64> }` is `#[repr(C)]` so the
    perf-tracked follow-up (RES-166 notes on RES-131 bounds
    elision) can load `len` at a stable offset without another
    ABI churn.
  - Shims use `extern "C-unwind"` (stable since 1.71) rather than
    plain `extern "C"`. Byte-for-byte identical ABI on every
    target cranelift emits, but lets panics propagate so Rust
    `#[should_panic]` tests can exercise the bounds-check paths
    without the process aborting.
  - New `register_runtime_symbols(&mut JITBuilder)` helper wires
    the four symbols by absolute address. Called from
    `make_module` AFTER `JITBuilder::with_isa` and BEFORE
    `JITModule::new` (module symbol tables freeze on
    construction). Extracted into its own fn so RES-167
    (builtin-call lowering) can reuse the registration seam.
  - `make_module`: one-line change to make the builder `mut` and
    call the new helper.
- Fifteen new unit tests named `res166a_*` cover:
  - `res_array_new` non-null for positive, zero, and negative
    lens (clamp behaviour).
  - Zero-initialization of all elements.
  - `set` / `get` round-trip across several slots.
  - `set` overwrites a previous value.
  - `#[should_panic]` guards: OOB on `get`, negative-index on
    `get`, OOB on `set`, null-ptr on `get`, null-ptr on `set`.
  - `res_array_free(null)` is a no-op.
  - Module-construction regression guard: `make_module` still
    succeeds after symbol wiring.
  - End-to-end JIT path regression: `run(parse("return 2+3;"))`
    still returns 5 with the new wiring in place.
  - Large-array sanity: 100-slot identity `a[i] = i*2`.

### Verification

```
$ cargo build                        # OK (8 warnings, baseline)
$ cargo build --features jit         # OK
$ cargo test --locked
test result: ok. 611 passed; 0 failed   (non-jit baseline unchanged)
$ cargo test --locked --features jit
test result: ok. 712 passed; 0 failed   (+15 vs previous 697)
$ cargo test --features jit res166a
test result: ok. 15 passed; 0 failed
```

### What was intentionally NOT done

- **RES-166b** — no `Node::ArrayLiteral` lowering, no
  `Node::IndexExpression` lowering. Both still return
  `JitError::Unsupported`.
- **RES-166c** — no `Node::IndexAssignment` lowering.
- **RES-166d** — no `benchmarks/jit/` addition, no parity smoke
  test between JIT and tree-walker on a summed array.
- No changes to the existing calling convention or to any
  lowering path beyond the `make_module` one-liner.

### Follow-ups the Manager should mint

- **RES-166b** — JIT lowering for `Node::ArrayLiteral` (call
  `res_array_new` + `res_array_set` per element) and for
  `Node::IndexExpression` (call `res_array_get`).
- **RES-166c** — JIT lowering for `Node::IndexAssignment` (call
  `res_array_set`).
- **RES-166d** — `benchmarks/jit/array_sum.rs` + smoke test that
  asserts JIT-vs-tree-walker parity.

## Attempt 1 failed

Oversized: this ticket introduces the JIT's first runtime FFI.

- Three `extern "C"` shim fns (`res_array_new`, `res_array_get`,
  `res_array_set`) with a concrete `*mut Array` repr.
- Symbol registration in `make_module`
  (`JITBuilder::symbol(...)`) — the existing backend has **no**
  FFI symbol wiring today (`grep -n "\.symbol(" src/jit_backend.rs`
  → 0 hits).
- Cranelift declarations + `call_indirect` through the imported
  addresses.
- Lowering arms for `Node::IndexExpression` and
  `Node::IndexAssignment`; both absent from the JIT today (`grep
  -n "IndexExpression\|ArrayLiteral" src/jit_backend.rs` → 0 hits).
- A bench in `benchmarks/jit/` + a smoke test for OOB.

Each slice is iteration-sized on its own on top of an unfamiliar
Cranelift backend.

## Clarification needed

Manager, please split:

- RES-166a: add `mod runtime_shims` inside `src/jit_backend.rs` with
  the three `res_array_*` fns, wire `JITBuilder::symbol(...)` in
  `make_module`. Test by calling the shims from a synthetic
  Cranelift IR fn.
- RES-166b: JIT lowering for `Node::ArrayLiteral` + the read path
  (`Node::IndexExpression`) built on 166a.
- RES-166c: JIT lowering for the write path
  (`Node::IndexAssignment`).
- RES-166d: the benchmark + smoke test asserting JIT-vs-tree-walker
  parity on an array-sum loop.

No code changes landed — only the ticket state toggle and this
clarification note.
