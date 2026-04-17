---
id: RES-012
title: Support prefix operators `!` and `-`
state: DONE
priority: P1
goalpost: G4
created: 2026-04-16
owner: executor
---

## Summary
Prefix `!` (logical not) and prefix `-` (unary minus) are common enough
in real programs that their absence is visible: all four self-healing
and sensor examples use them (`!toggle`, `return -1`) and currently
surface as `Token::Unknown('!')` or fail silently. We have the
`Node::PrefixExpression` variant already sitting unused in the AST.

## Acceptance criteria
- `!true` parses into `PrefixExpression { operator: "!", right: Bool(true) }`
- `-5` parses into `PrefixExpression { operator: "-", right: Int(5) }`
- `let x = !done;` and `let y = -1;` parse and evaluate
- Interpreter implements both: bool-not and numeric negation
- `Token::Bang` (or reuse `Token::Unknown`? no, proper token) added to lexer
- Unit tests cover each operator type; interpreter tests too
- At least one of the previously-broken examples (self_healing2.rs) makes
  meaningful additional progress with this ticket landed

## Notes
- Node::PrefixExpression already has `#[allow(dead_code)]` — remove it.
- Prefix precedence is typically higher than infix. Check the Pratt
  table in `parse_expression` / `current_precedence`.
- `!` conflicts with the `!=` prefix — lexer already handles that fork
  in the `'!'` arm; adjust the non-`=` branch to emit `Token::Bang`
  instead of `Token::Unknown('!')`.

## Log
- 2026-04-16 created by manager
