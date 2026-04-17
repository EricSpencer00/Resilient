---
id: RES-146
title: Trig / log / exp math builtins (sin cos tan ln log exp)
state: OPEN
priority: P3
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
Embedded control work needs basic transcendentals — filter
coefficients, PID tunings, unit conversions. All return Float;
Int inputs are widened via explicit `to_float` (per RES-130's
policy).

## Acceptance criteria
- Six builtins registered:
  - `sin(Float) -> Float`
  - `cos(Float) -> Float`
  - `tan(Float) -> Float`
  - `ln(Float) -> Float` — natural log
  - `log(Float, Float) -> Float` — log base_b of x (first arg is
    base, second is value — matches "log_b x" math notation).
    Runtime error on non-positive args or base == 1.
  - `exp(Float) -> Float` — e^x
- Implementation uses `f64` methods directly. No special-casing
  NaN / inf; they propagate.
- Unit tests assert values to 1e-9 precision against known
  references.
- Gate behind std for now. The no_std runtime uses `libm` via a
  follow-up ticket; don't block on that here.
- Commit message: `RES-146: trig / log / exp builtins`.

## Notes
- Argument order for `log`: we picked base first to match "log base
  2 of N" English phrasing. Rust's `f64::log(base)` puts base
  second — document the difference to avoid surprising anyone
  coming from Rust.
- Don't add `atan2` yet — separate ticket. The six listed cover
  90% of what users ask for.

## Log
- 2026-04-17 created by manager
