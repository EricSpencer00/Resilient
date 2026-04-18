---
id: RES-146
title: Trig / log / exp math builtins (sin cos tan ln log exp)
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`: six new transcendental builtins, all
  float-in / float-out per RES-130 (no implicit int↔float
  coercion — error messages hint at `to_float(x)` for users
  coming from looser languages).
  - `sin(Float)` / `cos(Float)` / `tan(Float)` — radians. Use
    `f64::sin/cos/tan` directly.
  - `ln(Float)` — rejects `x <= 0` with a dedicated message.
    `ln(0)` in raw f64 would return `-inf`, but the parallel
    `log` branch's ticket language says "Runtime error on
    non-positive args"; applying that to `ln` too is the
    coherent API.
  - `log(base: Float, value: Float)` — base-first argument
    order to match the English "log base 2 of 8" phrasing.
    The ticket's Notes specifically flag the difference from
    Rust's `f64::log(base)`. Rejects `base <= 0`, `base == 1`
    (log₁ is undefined), and `value <= 0` with distinct
    diagnostics.
  - `exp(Float)` — `f64::exp`. Overflow to `+inf` propagates
    per the ticket's "No special-casing NaN / inf".
- Wired all six into the `BUILTINS` table.
- `resilient/src/typechecker.rs`: added `fn(Float) -> Float`
  entries for sin/cos/tan/ln/exp and `fn(Float, Float) -> Float`
  for log. A local `fn_float_to_float()` closure keeps the five
  single-arg registrations concise.
- Deviations: none. Six builtins, float-only, errors on
  invalid-domain args, no special-casing of NaN/inf, std-only.
- Unit tests (13 new, all asserting values to `1e-9` via a
  shared `close()` helper):
  - `sin_cos_tan_zero` — identities at 0
  - `sin_cos_at_half_pi` — sin(π/2)=1, cos(π/2)≈0
  - `tan_pi_over_4` — tan(π/4)=1
  - `ln_of_e_and_one` — ln(e)=1, ln(1)=0
  - `ln_rejects_non_positive` — x=0 and x=-3
  - `ln_rejects_int_per_res130` — error hints at `to_float`
  - `log_base_2_of_8` — validates (base, value) ordering
  - `log_rejects_base_one`
  - `log_rejects_non_positive_base_and_value`
  - `exp_zero_one_and_ln_roundtrip` — exp(0)=1, exp(1)=e,
    ln(exp(2.5))≈2.5
  - `exp_rejects_non_float`
  - `sin_rejects_non_float`
  - `trig_log_exp_arity_errors` — arity coverage
- Verification:
  - `cargo test --locked` — 360 passed (was 347 before RES-146)
  - `cargo test --locked --features logos-lexer` — 361 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
