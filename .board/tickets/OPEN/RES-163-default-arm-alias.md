---
id: RES-163
title: `default =>` alias for `_ =>` in match arms
state: OPEN
priority: P3
goalpost: G13
created: 2026-04-17
owner: executor
---

## Summary
Small readability win. `default => ...` reads more like English
than `_ => ...`, and users coming from other C-family languages
expect the keyword. Both forms stay supported — this is pure
alias.

## Acceptance criteria
- Lexer adds `default` as a keyword.
- Parser accepts `default` wherever `_` is accepted at the top of
  a match arm; desugars to `_` at parse time so downstream
  phases are unchanged.
- `default` as an identifier now becomes a lex error — document
  as part of the feature (shadowing keywords was never allowed).
- Unit tests: `default => ...` arm exhausts a previously
  non-exhaustive match. `let default = 3;` errors.
- SYNTAX.md notes `default` as an alias.
- Commit message: `RES-163: default as _ alias in match arms`.

## Notes
- If any existing example / test uses `default` as an identifier,
  rename before merging. Check examples/ and all tests.
- Don't add `otherwise` or `else` as further aliases — one
  synonym is plenty.

## Log
- 2026-04-17 created by manager
