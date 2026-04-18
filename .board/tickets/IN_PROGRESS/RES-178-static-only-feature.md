---
id: RES-178
title: `static-only` feature: ban alloc at build time, enforce heap-free
state: IN_PROGRESS
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
