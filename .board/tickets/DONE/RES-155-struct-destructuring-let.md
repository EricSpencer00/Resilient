---
id: RES-155
title: Struct destructuring `let Point { x, y } = p`
state: DONE
priority: P3
goalpost: G12
created: 2026-04-17
owner: executor
---

## Summary
Destructuring a struct into local bindings is a common read
pattern. Pair with RES-154's shorthand so the same `{ x, y }` works
on both sides.

## Acceptance criteria
- Parser: `let <StructName> { field1, field2: local_name, .. } =
  expr;`. The `..` rest pattern allows ignoring trailing fields.
- Fields listed without `: name` bind to a local of the same name;
  with `: name` bind to an explicitly-renamed local.
- Exhaustiveness: without `..`, every field of the struct must
  appear — a typecheck error otherwise, listing missing fields.
- Unit tests: full destructure, renaming, rest pattern,
  non-exhaustive without `..`.
- Commit message: `RES-155: struct destructuring let`.

## Notes
- This is purely a let-binding feature — match arms get struct
  destructuring via RES-161.
- Don't support reference patterns / nested struct patterns in
  this ticket: one layer deep is enough to unblock most ergonomic
  wins. Deeper nesting is a follow-up.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - New `Node::LetDestructureStruct { struct_name, fields:
    Vec<(String, String)>, has_rest, value, span }` variant.
    `fields` carries `(field_name, local_name)` pairs;
    `local_name == field_name` for the shorthand form `{ x }`.
  - `parse_let_statement` detects the destructuring form via a
    single-token lookahead after the identifier: if
    `current_token == LeftBrace` (not `:` or `=`), it delegates
    to `parse_let_destructure_struct`. No ambiguity with the
    existing simple-let / annotated-let forms.
  - New `parse_let_destructure_struct(struct_name, span)`:
    walks field patterns (shorthand or renamed via `:`), handles
    the `..` rest token (two consecutive `Token::Dot` — no new
    lexer token needed since `..` only appears in this
    position), accepts trailing commas.
  - Interpreter eval arm evaluates the RHS, verifies
    `Value::Struct`, checks the struct name matches, then binds
    each requested field's value to the corresponding local.
    Clean diagnostics for non-struct values, wrong struct name,
    and missing fields.
- `resilient/src/typechecker.rs`:
  - New `Node::LetDestructureStruct` arm. If the struct is
    declared, validates:
    1. **Unknown field names first** — `Struct X has no field
       `y`` — because typos produce clearer diagnostics than
       the missing-field cascade they would otherwise generate.
    2. **Exhaustiveness** when `has_rest == false` — lists
       missing field names in sorted order with the suggestion
       "add `..` to ignore them".
  - Each local binding is entered into the env with the
    declared field's type, or `Any` when the struct declaration
    isn't visible (tolerant fallback).
- `resilient/src/compiler.rs`: `Node::LetDestructureStruct` arm
  added to `node_line`'s exhaustive span accessor.
- `SYNTAX.md`: new "Destructuring let (RES-155)" subsection
  under Structs with examples of full destructure, renaming,
  and the `..` rest pattern, plus a note about the one-layer-
  deep scope.
- Deviations: none.
- Unit tests (8 new):
  - `let_destructure_full_binds_every_field_shorthand`
  - `let_destructure_renames_field_to_local`
  - `let_destructure_rest_pattern_ignores_remaining_fields`
  - `let_destructure_mixed_shorthand_and_rename`
  - `let_destructure_non_exhaustive_without_rest_is_typecheck_error`
    — validates the "missing field(s) b, c" diagnostic shape
    required by the ticket.
  - `let_destructure_unknown_field_is_typecheck_error` — typo
    produces "Struct X has no field `y`".
  - `let_destructure_wrong_struct_name_is_runtime_error` — a
    `let Bar { a } = f;` where `f: Foo` fails at eval.
  - `let_destructure_non_struct_value_is_runtime_error` — an
    Int RHS produces a clean "Cannot destructure non-struct"
    message.
- Smoke (manual):
  - Full: `let Point { x, y } = p` → bindings `3`, `4`.
  - Rename: `let Point { x: a, y: b } = p` → `a=3`, `b=4`.
  - Rest: `let Foo { a, .. } = f` → `a=1`, `b/c` ignored.
  - Non-exhaustive w/o `..`: typecheck error
    `Non-exhaustive destructure of Foo: missing field(s) b, c`.
- Verification:
  - `cargo test --locked` — 413 passed (was 405 before RES-155)
  - `cargo test --locked --features logos-lexer` — 414 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
