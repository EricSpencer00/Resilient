---
id: RES-098
title: resilient-runtime adds embedded-alloc + Float/String Value variants
state: DONE
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
- 2026-04-17 executor landed:
  - `resilient-runtime/Cargo.toml`: new `alloc` feature
    (default off) + optional `embedded-alloc = "0.5"` dep gated
    under it.
  - `lib.rs`:
    - `#[cfg(feature = "alloc")] extern crate alloc;` plus
      `use alloc::string::String;`. Needed in BOTH test and
      production builds even though std re-exports alloc — the
      `extern crate` is the bit that makes it linkable.
    - `Value` gains `Float(f64)` (always available — no allocator
      needed) and `String(alloc::string::String)` (gated on
      `alloc`). Dropped `Eq` derive (f64 lacks it) and
      `Copy` derive (String can't be Copy).
    - `Value::add/sub/mul/div/eq` extended:
      - Float arithmetic uses native f64 ops; no overflow concept.
      - Float div produces inf/NaN per IEEE-754 (no error).
      - Float eq uses `to_bits` so NaN equals itself (matches the
        bytecode VM's constant-pool dedup).
      - String + String concatenates (alloc only).
      - String + String == compares as Eq.
      - Mixed int/float and int/string remain TypeMismatch.
- 2026-04-17 tests:
  - 7 RES-075 tests still pass.
  - 4 new RES-098 always-on Float tests cover arithmetic,
    division-by-zero-yields-inf, NaN-equals-itself, mixed-with-int
    rejection.
  - 3 alloc-gated tests cover string concat, string eq,
    string-doesn't-subtract.
  - Total: 11 tests on default features, **14 tests on `--features alloc`**.
- 2026-04-17 verification across four config combinations:
  - host (default, no alloc): build/test/clippy clean
  - host `--features alloc`: build/test/clippy clean
  - cross (`--target thumbv7em-none-eabihf`, no alloc): build/clippy clean
  - cross `--target thumbv7em-none-eabihf --features alloc`:
    build/clippy clean (pulls in `linked_list_allocator` +
    `critical-section` + `embedded-alloc` transitively)
- README "Embedded runtime" section updated with both feature
  configs + a Cortex-M `#[global_allocator]` example showing how
  embedded users wire `embedded_alloc::LlffHeap`.
- The lib does NOT pick a `#[global_allocator]` — that's the
  binary's responsibility. Keeps `resilient-runtime` reusable
  across allocator choices.
