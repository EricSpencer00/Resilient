---
id: RES-039
title: match expressions with literal, identifier, and wildcard patterns
state: DONE
priority: P1
goalpost: G13
created: 2026-04-16
owner: executor
---

## Summary
Structured dispatch. Chained if/else is already painful at 3+ branches;
with structs it's miserable. `match` is the standard way.

## Acceptance criteria

    match value {
        0 => println("zero"),
        1 => println("one"),
        n => println("other: " + n),
    }

- Literal patterns: int, float, string, bool literals match by equality
- Identifier pattern: binds the value, always matches (acts as default)
- Wildcard `_`: matches anything without binding
- Arms are `PATTERN => EXPR,` — trailing comma permitted
- Exhaustiveness NOT required at MVP (a fallthrough without match
  produces `Value::Void`); proper exhaustiveness checking comes with G7
- match can appear as a statement; its value is the matched arm's
  result
- Tests: all three pattern types, fall-through to wildcard, no-match
  returns Void

## Notes
- New Token::Match, Token::FatArrow (`=>`), Token::Underscore (`_`).
- New Node::Match { scrutinee, arms: Vec<(Pattern, Node)> }.
- New Pattern enum { Literal(Node), Identifier(String), Wildcard }.
- Parser handles `match EXPR { ARM, ARM, ... }`.
- Interpreter walks arms in order, returns the first match's result.

## Log
- 2026-04-16 created by manager
- 2026-04-16 claimed by executor
