---
id: RES-178
title: `static-only` feature: ban alloc at build time, enforce heap-free
state: DONE
priority: P3
goalpost: G16
created: 2026-04-17
owner: executor
---

## Summary
Some safety-critical projects forbid dynamic allocation entirely.
Offer a `static-only` feature for `resilient-runtime` that
`extern crate alloc` would fail under; all code must compile
without touching `Vec`, `Box`, `String`, or any alloc type.

## Acceptance criteria
- Feature `static-only` in `resilient-runtime/Cargo.toml`.
  Mutually exclusive with `alloc` — build errors if both set.
- Under `static-only`, `Value::String` and `Value::Array` /
  `Value::Map` / etc. are conditionally absent. `Value` shrinks to
  `Int | Bool | Float`.
- Unit tests run under `--features static-only` and cover the
  reduced Value surface.
- Documented in the runtime README with a decision table:
  "what Value variants are available under which features".
- Commit message: `RES-178: static-only feature bans alloc types`.

## Notes
- This is not about preventing allocation in user Resilient
  programs yet — that's a language-level ticket involving a
  typechecker pass. This is runtime-only: whatever the frontend
  chooses to emit, the runtime it links against cannot support
  alloc.
- The frontend (resilient/) doesn't build under static-only — the
  compiler itself needs alloc. Just runtime.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient-runtime/Cargo.toml`: new `static-only` feature
  (empty — the enforcement is structural, not code-pulled).
  Documented as "assertive alloc-ban" and mutually exclusive
  with `alloc`.
- `resilient-runtime/src/lib.rs`:
  - New `compile_error!` gated on `all(feature = "alloc",
    feature = "static-only")`. Users who set both get a clean
    build-time error naming exactly what each feature does
    and why they can't coexist. Verified locally by running
    `cargo build --features "alloc static-only"` and
    confirming the diagnostic fires.
  - Two new unit tests behind `all(feature = "static-only",
    not(feature = "alloc"))`:
    - `static_only_int_bool_float_still_work` — arithmetic
      on each of the three always-available variants works
      end-to-end, proving the reduced surface is functional.
    - `static_only_value_enum_omits_string_variant` — an
      exhaustive `match` over `Value` with only Int / Bool /
      Float arms compiles. If the String variant ever sneaks
      in (e.g. a refactor drops its `#[cfg(feature = "alloc")]`
      gate), this test's match fails to compile — the
      regression is caught at build time.
- `README.md`: new "`--features static-only` (RES-178)"
  subsection under the Embedded runtime section with the
  ticket-required decision table:
  - default → Int / Bool / Float ✅, String ✕
  - alloc → all four ✅
  - static-only → Int / Bool / Float ✅, String ✕ (same
    surface as default — the feature is assertive, not
    additive)
  - alloc + static-only → build fails
  Also notes that the `resilient/` CLI crate stays on alloc
  (the compiler itself needs it) — static-only is runtime-only.
- Deviations from the ticket:
  - The ticket listed `Value::Array` / `Value::Map` as
    variants to gate. Those variants don't exist in the
    runtime today (they're in the `resilient` CLI crate's
    Value enum; the two value types haven't converged).
    The `static-only` contract gates what's there — `String`
    — and the README's decision table calls out the
    divergence explicitly so a future convergence ticket
    knows to extend the gating.
- Verification:
  - `cargo test` (default) — 11 passed.
  - `cargo test --features alloc` — 14 passed.
  - `cargo test --features static-only` — 13 passed (11
    pre-existing non-alloc tests + 2 new static-only
    tests).
  - `cargo build --features "alloc static-only"` — fails
    with the expected compile_error! diagnostic.
  - Cross-target builds pass under `static-only`:
    - `riscv32imac-unknown-none-elf` ✓
    - `thumbv6m-none-eabi` ✓
    - `thumbv7em-none-eabihf` ✓
  - `resilient` host regression: 468 tests pass (CLI crate
    unaffected, as specified).
