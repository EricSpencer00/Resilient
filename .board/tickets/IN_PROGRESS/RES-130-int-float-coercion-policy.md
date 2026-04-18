---
id: RES-130
title: Decide and document int ↔ float coercion policy
state: OPEN
priority: P3
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
Today `1 + 2.0` produces a float silently — the interpreter coerces.
A safety-critical language should be deliberate here. This ticket
picks a policy, documents it, and adds tests pinning the behavior.
Recommendation: **no implicit coercion**; require `to_float(x)` or
`to_int(x)` at the boundary.

## Acceptance criteria
- SYNTAX.md gets a "Numeric coercion policy" section stating: no
  implicit conversions between Int and Float, ever. Mixed operands
  are a type error.
- Typechecker enforces the rule. The interpreter's silent coercion
  path removed.
- New builtins: `to_float(Int) -> Float`, `to_int(Float) -> Int`
  (latter truncates; document clearly).
- Existing examples updated if they relied on implicit coercion
  (check `sensor_monitor.rs`).
- Unit tests: one per operator × mixed-types combo = error;
  explicit `to_float` + mixed-is-now-same-type = success.
- Commit message: `RES-130: no implicit int↔float coercion`.

## Notes
- This IS a breaking change for any user code that relied on the
  old behavior. Acceptable pre-1.0; call it out in the roadmap
  changelog.
- `to_int(Float)` semantics: truncate toward zero. `to_int(NaN)`
  and `to_int(±inf)` are runtime errors with clean messages.

## Log
- 2026-04-17 created by manager
