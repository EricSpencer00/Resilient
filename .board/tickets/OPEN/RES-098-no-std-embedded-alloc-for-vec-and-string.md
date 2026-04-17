---
id: RES-098
title: resilient-runtime adds embedded-alloc + Float/String Value variants
state: OPEN
priority: P3
goalpost: G16
created: 2026-04-17
owner: executor
---

## Summary
RES-075 Phase A's `Value` carries only `Int(i64)` and `Bool(bool)`
because Float (well, that's stack-only — actually keeping Float
needs no allocator; the real driver is String/Array which DO need
heap allocation). This ticket adds `embedded-alloc` as an opt-in
dep, an `alloc` feature flag, and grows `Value` with `Float(f64)`
and `String(alloc::string::String)` variants when `alloc` is on.

The host build keeps the alloc-free Value subset by default; a
build with `--features alloc` (and on embedded, an allocator
selected at link time) opts into the richer variants.

## Acceptance criteria
- `resilient-runtime/Cargo.toml` adds an `alloc` feature, plus
  `embedded-alloc = "0.5"` (or current latest) as an optional dep
  gated under it.
- `resilient-runtime/src/lib.rs`:
  - `extern crate alloc;` under `#[cfg(feature = "alloc")]`.
  - New variants `Value::Float(f64)` (always available — no alloc
    required) and `Value::String(alloc::string::String)` (gated
    on `alloc`).
  - `Value::add` etc. extended:
    - `Float + Float` adds wrap-free (f64 has no overflow).
    - `String + String` concatenates (gated on alloc).
    - Mixed Int + Float is a TypeMismatch — promotion is the
      caller's job.
- New unit tests cover float add/sub/mul/div, string concat
  (under `--features alloc`), mixed-type rejections.
- Default feature set (no `alloc`) still builds on host AND on
  `thumbv7em-none-eabihf`.
- `--features alloc` builds on host + tests pass.
- `--features alloc` cross-compile on thumbv7em-none-eabihf
  builds (the LLFF heap from `embedded-alloc` is selected only
  when the user wires `#[global_allocator]`; the lib itself
  doesn't pick one — that's RES-099's example).
- README "Embedded runtime" section gains a paragraph explaining
  the `alloc` feature.
- Commit message: `RES-098: resilient-runtime + embedded-alloc — Float/String values`.

## Notes
- Don't pick a `#[global_allocator]` in the lib — that's an
  application-level decision. The lib only needs to know
  `extern crate alloc;` is available.
- Embedded users will write something like:
  ```rust
  use embedded_alloc::LlffHeap as Heap;
  #[global_allocator]
  static HEAP: Heap = Heap::empty();
  ```
  in their binary's `main()`. RES-099 will demo that.
- Blocked on RES-097 — need the cross-compile path proven before
  layering alloc on top.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
