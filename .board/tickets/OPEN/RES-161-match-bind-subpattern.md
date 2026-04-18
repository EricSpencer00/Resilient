---
id: RES-161
title: Bind-subpattern `case p @ Point { x, y } if x > 0 => ...`
state: OPEN
priority: P3
goalpost: G13
created: 2026-04-17
owner: executor
---

## Summary
The `name @ pattern` form lets the user bind the whole matched
value AND destructure it in the same arm. Useful for guards and
for forwarding the whole value to another function.

## Acceptance criteria
- Parser: `<name> @ <pattern>` at pattern position.
- Semantics: `name` binds the full value; inner pattern binds its
  parts. Both bindings are in scope in guards and arm bodies.
- Nested: `a @ Point { x: b @ Int, y }` — allowed but recursive
  only one level for this ticket (no `a @ (b @ ...)` chains).
- Unit tests covering bind-then-destructure with struct, tuple,
  and integer literal patterns; guard access to both `p` and
  `x`.
- Commit message: `RES-161: bind-subpattern @ in match`.

## Notes
- Type of `name` is the outer scrutinee's type; pattern bindings
  get their inferred types as usual.
- Mnemonic: the `@` reads as "also named" — document inline.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (blocked on missing
  struct-in-match and tuple-in-match patterns)

## Attempt 1 failed

Bailing: three stacked blockers, any one of which would already
push this outside one iteration.

1. **Struct patterns don't exist in match arms.** The
   `case p @ Point { x, y } => ...` form the ticket title uses
   requires `Pattern::Struct { name, fields }` to exist. Today
   `Pattern` is `Literal | Identifier | Wildcard | Or` (see
   `resilient/src/main.rs`); there is no struct-shape match.
   RES-155 added struct destructuring for `let`, deliberately
   scoped away from match arms per its own Notes:
   "This is purely a let-binding feature — match arms get struct
   destructuring via RES-161." That implies the struct-in-match
   work was expected to land **as part of** RES-161; the ticket's
   acceptance-criteria bullet "Unit tests covering bind-then-
   destructure with struct, tuple, and integer literal patterns"
   confirms that intent.

2. **Tuple patterns don't exist in match arms** — and neither do
   tuples as values. RES-127 (tuple types) is itself bailed and
   open with a `## Clarification needed` note listing a split
   into four sub-tickets. Without a tuple value type there's
   nothing for a tuple pattern to destructure.

3. **Bind-subpattern over just Literal / Identifier / Wildcard /
   Or is marginally useful.** The compelling use cases the
   ticket's Summary lists ("forwarding the whole value to
   another function", nested `Point { x: b @ Int, y }`) all
   depend on having structural patterns to wrap. With only the
   existing flat patterns, we'd end up with:
   - `x @ _` ≡ `x` (Identifier alone) — redundant
   - `x @ 5` — binds `x` to the scrutinee AND only matches on
     5; `x` is always 5 in the body so the bind is useless
   - `x @ (p1 | p2)` — combines with RES-160's or-patterns;
     usable but narrow
   Delivering just that slice would be a scope reduction the
   acceptance criteria don't authorize (they require struct +
   tuple coverage).

Shipping the full ticket as written is a multi-iteration
undertaking:

- Pattern::Bind(String, Box<Pattern>) variant + parser
  `<name> @ <pattern>` support
- Pattern::Struct { name, fields, rest } variant + parser
  mirror of RES-155's destructure
- Exhaustiveness / binding-consistency updates across all
  three new patterns
- Interpreter + typechecker arms for each
- Pattern::Tuple { items } — blocked on RES-127 entirely

Each of these is a self-contained iteration.

## Clarification needed

Manager, please sequence:

- **RES-161a**: `Pattern::Bind(String, Box<Pattern>)` plus the
  `<name> @ <pattern>` parse rule. Inner pattern restricted to
  what exists today (Literal / Identifier / Wildcard / Or).
  Acceptance criteria downscoped to "integer literal + or-
  pattern" — drop the struct / tuple bullets. Independently
  useful once RES-161b lands.
- **RES-161b**: `Pattern::Struct { name, fields, has_rest }` in
  match arms. Mirrors RES-155's let-destructure (same exhaustive
  field rule, same `..` rest support, same unknown-field
  diagnostic). Enables `case Point { x, y } => ...`.
- **RES-161c** (gated on RES-127): `Pattern::Tuple { items }`
  in match arms. Can't be written until tuples are a value type.
- **RES-161d**: re-open this ticket with the full
  struct-and-tuple coverage once a–c land.

Landing 161a by itself is a clean, small slice. 161b unblocks a
huge ergonomic win (destructure in match) that other tickets are
currently waiting for. Without that sequencing the bind-
subpattern sugar doesn't justify its own ticket.

No code changes landed — only the ticket state toggle and this
clarification note. Committing as a ticket-only move so `main`
stays unchanged except for the metadata.
