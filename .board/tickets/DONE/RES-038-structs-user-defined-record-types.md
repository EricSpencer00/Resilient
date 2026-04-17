---
id: RES-038
title: Structs — user-defined record types
state: DONE
priority: P0
goalpost: G12
created: 2026-04-16
owner: executor
---

## Summary
Phase 2 opens with structs — the single feature that makes it
possible to write domain programs (sensor readings, PID controllers,
state machines) without smuggling state through parallel arrays.

## Acceptance criteria

Declaration:

    struct Point {
        int x,
        int y,
    }

Construction (named-field literal):

    let p = Point { x: 3, y: 4 };

Field access:

    let dx = p.x;

Field update (immutable — returns new struct? or mutable via
dot-assignment? pick mutable for MVP, matching array index
assignment from RES-032):

    p.x = 5;

- Zero-field structs allowed (`struct Empty {}`).
- Field access chains: `box.pos.x` works.
- Tests covering declaration, construction, access, mutation, chains.
- Error cases: wrong field name, missing field on construction.

## Notes
- New tokens: Token::Struct, Token::Dot (`.`).
- New AST nodes: Node::StructDecl, Node::StructLiteral, Node::FieldAccess,
  Node::FieldAssignment.
- New Value::Struct { name: String, fields: Vec<(String, Value)> } —
  use Vec not HashMap so field order is stable for Display.
- Parser: `struct NAME { TYPE FIELD, ... }` registered via the
  statement dispatcher alongside `fn`.
- The `.` token was previously only a float-point part; it's now a
  real token at the statement level. Make sure the `0.5` lexer path
  still works (it does — read_number sees the digit first and
  consumes the `.` before the tokenizer can misroute).

## Log
- 2026-04-16 created by manager
- 2026-04-16 claimed by executor
