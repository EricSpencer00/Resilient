---
id: RES-014
title: Fix Pratt parser current-token invariant
state: DONE
priority: P1
goalpost: G4
created: 2026-04-16
owner: executor
---

## Summary
`parse_expression` leaves `current_token` pointing at the LAST token
the expression consumed, not the token AFTER. Callers like
`parse_if_statement` then check `current_token != LeftBrace` and fail
for any expression that doesn't end on an identifier or literal
that's immediately followed by `{`. The concrete symptom from
`examples/self_healing2.rs:7`:

    if read_random(0) < 0.5 {
      ^                    ^
      ok                   parser says "Expected '{' here"
                           but current_token is FloatLiteral(0.5)

## Acceptance criteria
- `if call_expr() < 0.5 { ... }` parses without error
- `if x == 0 { ... }` still parses
- Regression test: `parse("fn f() { if g(0) < 0.5 { } }")` has no errors
- At least one previously-broken example
  (self_healing2 / sensor_example2) runs further than it does today —
  ideally to interpretation

## Notes
- The fix is probably one of:
  1. `parse_expression` advances `current_token` past the last expression
     token before returning, OR
  2. all callers use `peek_token` to decide the next action.
- Either way, pick the invariant and hold it consistently. Document it
  as a comment on `parse_expression`.
- Touches `parse_let_statement`, `parse_return_statement`, and
  `parse_if_statement` in addition to `parse_expression`.

## Log
- 2026-04-16 created by manager
