---
id: RES-163
title: `default =>` alias for `_ =>` in match arms
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - New `Token::Default` variant + Display string `` `default` ``.
  - Hand-rolled lexer keyword map: `"default" => Token::Default`
    (alongside the existing `"_" => Token::Underscore` arm).
  - `parse_pattern_atom` accepts `Token::Default` as
    `Pattern::Wildcard`, exactly the same AST the `_` branch
    produces. No downstream phase (typechecker, interpreter,
    VM, JIT, compiler) needs to change — the desugar happens at
    parse time as the ticket requires.
- `resilient/src/lexer_logos.rs`:
  - New `#[token("default")] Default` tok (positioned before
    `Ident` so logos picks the keyword arm).
  - `convert` arm maps `Tok::Default` → `Token::Default`.
- Pre-work audit: `grep` confirmed no example / test / SYNTAX
  occurrence uses `default` as an identifier, so no migration
  was needed. The ticket's "rename before merging" check ran
  clean.
- `SYNTAX.md`: new "`default` keyword (RES-163)" subsection
  under Match expressions documenting the alias, showing the
  classify-with-default example, and calling out the
  identifier-shadowing rejection with the exact error shape
  users will see.
- Deviations: none.
- Unit tests (5 new):
  - `default_arm_exhausts_previously_non_exhaustive_match` —
    ticket AC: match on Int with `default =>` both typechecks
    and runs.
  - `default_and_underscore_are_interchangeable_at_match_position`
    — parallel `_` and `default` programs produce identical
    output.
  - `default_as_let_binding_name_is_a_parse_error` — ticket AC.
  - `default_in_arbitrary_expression_position_is_a_parse_error`
    — generalizes the identifier-rejection invariant beyond
    just `let`.
  - `default_works_inside_or_pattern_and_with_guards` —
    regression: the new token composes with RES-159 guards and
    RES-160 or-patterns without special-casing.
- Verification:
  - `cargo test --locked` — 445 passed (was 440 before RES-163)
  - `cargo test --locked --features logos-lexer` — 446 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
