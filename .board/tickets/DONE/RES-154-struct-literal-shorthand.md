---
id: RES-154
title: Struct literal shorthand `Point { x, y }`
state: DONE
priority: P3
goalpost: G12
created: 2026-04-17
owner: executor
---

## Summary
`Point { x: x, y: y }` is tiresome when the local variable name
matches the field name. Teach the parser to accept the shorthand
`Point { x, y }` — desugars to the full form before the
typechecker sees it.

## Acceptance criteria
- Parser: in struct-literal field position, an identifier with no
  following `:` expands to `name: name`. Mixing with explicit
  `other: expr` in the same literal works.
- Error if the shorthand name isn't bound as a local: usual "unknown
  identifier" diagnostic (the desugared form produces it naturally).
- Unit tests: pure shorthand, mixed shorthand + explicit, unbound
  identifier error.
- SYNTAX.md "Structs" section gets a shorthand example.
- Commit message: `RES-154: struct-literal field shorthand`.

## Notes
- Desugaring happens in the parser, not a later pass — keeps the
  typechecker, interpreter, VM, and JIT ignorant of the sugar.
- Don't add field-punning to struct patterns yet — that's covered
  under RES-155 destructuring.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - `parse_struct_literal`'s inner loop now checks, after reading
    the field-name identifier, whether the next token is `,` or
    `}`. If so, we synthesize a `Node::Identifier { name, span }`
    value — carrying the original field-name token's span — and
    push `(name, Identifier(name))` into the fields vector. The
    existing `,` / `}` handling picks up from there, so trailing
    commas and mixed explicit/shorthand fields both work.
  - Desugaring lives entirely in the parser per the ticket Notes:
    the AST that flows downstream is indistinguishable from the
    explicit form `Point { x: x, y: y }`. Typechecker, interpreter,
    VM, JIT, and compiler see nothing new. Unbound shorthand names
    surface through the normal `Identifier not found` diagnostic.
- `SYNTAX.md`:
  - New `## Structs` section with an example covering all three
    forms (explicit, pure shorthand, mixed), plus a sentence
    explaining the sugar-in-parser semantics.
  - "Data Types" table gains the `bytes` row (RES-152 follow-up).
- Deviations: none.
- Unit tests (5 new):
  - `struct_literal_shorthand_desugars_to_field_name_identifier`
    — pure shorthand `Point { x, y }` produces two
    `(name, Identifier(name))` pairs.
  - `struct_literal_shorthand_mixed_with_explicit_field`
    — `{ x, y: z }` — first shorthand, second explicit.
  - `struct_literal_shorthand_explicit_then_shorthand`
    — `{ x: 7, y }` — order-flipped variant.
  - `struct_literal_shorthand_with_trailing_comma`
    — `{ x, y, }` parses clean.
  - `struct_literal_shorthand_unbound_name_errors_at_runtime`
    — unbound shorthand produces `Identifier not found` per
    the ticket's acceptance criterion.
- Smoke (manual):
  - `new Point { x, y }` / `new Point { x, y: z }` both work
    end-to-end with expected field values.
  - `new Point { x, y }` with no bindings prints
    `Runtime error: Identifier not found: x` — the natural
    unbound-identifier diagnostic.
- Verification:
  - `cargo test --locked` — 405 passed (was 400 before RES-154)
  - `cargo test --locked --features logos-lexer` — 406 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
