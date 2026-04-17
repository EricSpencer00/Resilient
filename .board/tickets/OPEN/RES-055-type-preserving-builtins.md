---
id: RES-055
title: Type preserving signatures for math builtins
state: OPEN
priority: P2
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
`abs`, `min`, and `max` already preserve type (Int→Int, Float→Float)
but `pow`, `floor`, and `ceil` always coerce to `Value::Float` even
when the input is `Value::Int`. That's a footgun for an embedded /
no_std target where every float is a soft-float library call —
`floor(7)` should not magically become a `7.0`. This ticket makes the
math builtins keep the input type when doing so is lossless.

`sqrt` is intentionally **out of scope** here — `sqrt` of an int is
generally irrational, so `Float` is the right return type.

## Acceptance criteria
- `floor(int)` returns `Value::Int` (the input passes through unchanged); `floor(float)` keeps returning `Value::Float`.
- `ceil(int)` returns `Value::Int` (same passthrough); `ceil(float)` keeps returning `Value::Float`.
- `pow(int, int)` returns `Value::Int`. Use checked arithmetic (`i64::checked_pow`) and surface a clean `RResult` error on overflow rather than panicking. Negative-exponent on int args is a runtime error (`pow: negative exponent {exp} undefined for int base`). Mixed `pow(int, float)` or `pow(float, int)` keeps the existing float behavior.
- `sqrt` unchanged (still always Float).
- `abs`, `min`, `max` unchanged (already type-preserving).
- New unit tests in `resilient/src/main.rs` `#[cfg(test)] mod tests` covering: `floor(7)` → `Value::Int(7)`, `ceil(-3)` → `Value::Int(-3)`, `pow(2, 10)` → `Value::Int(1024)`, `pow(2, 63)` → overflow error, `pow(2, -1)` → negative-exponent error, `pow(2.0, 3)` → `Value::Float(8.0)`.
- A new `examples/int_math.rs` exercising `let p = pow(2, 8); println(p);` and asserting against an `int_math.expected.txt` golden — proves the int-purity end-to-end.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all pass.
- Commit message: `RES-055: type-preserving math builtins (floor/ceil/pow)`.

## Notes
- Builtins live in `resilient/src/main.rs` around `:2321` (sqrt) through `:2366` (ceil); registration table at `:2250`. `pow` at `:2331`, `floor` at `:2349`, `ceil` at `:2358`.
- `Value` enum is at `:1846`. `Value::Int(i64)`, `Value::Float(f64)`.
- Overflow guard for `pow`: convert `exp: i64` → `u32` via `try_into` (returning the negative-exponent error if it fails for negative numbers), then `base.checked_pow(exp_u32)`. Returns `Option<i64>`; map `None` to a clean error string.
- The typechecker's `fn_any_to_any` signature for these builtins (`typechecker.rs:296`) is already permissive — no typechecker change required for the new int returns to flow through.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
