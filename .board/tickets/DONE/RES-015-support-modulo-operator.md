---
id: RES-015
title: Support modulo operator `%`
state: DONE
priority: P2
goalpost: G4
created: 2026-04-16
owner: executor
---

## Summary
`sensor_example2.rs` uses `x % 2` for toggling behavior; lexer emits
`Token::Unknown('%')` today. Low-effort ticket: add the token, wire
it into the infix table and the int/float evaluators.

## Acceptance criteria
- `5 % 3` evaluates to `Value::Int(2)`
- `5.0 % 2.0` evaluates to `Value::Float(1.0)`
- Unit test covers both
- `%` has the same precedence as `*` and `/` (level 5)
- `sensor_example2.rs` makes measurable additional progress

## Log
- 2026-04-16 created by manager
