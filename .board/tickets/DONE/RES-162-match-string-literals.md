---
id: RES-162
title: Match against string literal patterns
state: DONE
priority: P3
goalpost: G13
created: 2026-04-17
owner: executor
---

## Summary
`case "hello" => ...` is natural for text-dispatch code (option
parsing, protocol codes, small state machines). Adds no new
algorithmic complexity — string equality at each arm.

## Acceptance criteria
- Parser: string literal at pattern position.
- Exhaustiveness: over the implicit infinite space of String, a
  literal-only match is never exhaustive without `_` — same rule
  as Int today.
- Unit tests covering success, fallthrough to `_`, escape handling
  in the literal pattern (`"a\n"` matches the same string).
- Commit message: `RES-162: string-literal match patterns`.

## Notes
- Don't introduce regex patterns here — that's a separate
  decision the language hasn't made yet, and it pulls in a
  runtime dependency.
- The interpreter, VM, and JIT all already handle string equality;
  match compilation just emits the same sequence of
  `if s == "pat"` checks.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor (feature was already wired; ticket
  closed via test-coverage-only work)

## Resolution
- **No production-code changes required.** Every acceptance
  criterion was already met by existing machinery:
  - Parser already accepts `Token::StringLiteral` in pattern
    position (see `parse_pattern_atom` — added alongside
    Int/Float/Bool literal patterns in RES-054 and extended in
    RES-160).
  - Interpreter's `match_pattern` already compares via
    `(Value::String(a), Value::String(b)) => a == b` — same
    path Int/Float/Bool literals use, so string equality "just
    works" with no new opcodes.
  - Typechecker's exhaustiveness already flags a literal-only
    string match as non-exhaustive — the "Non-exhaustive match
    on {type}" branch fires for any scrutinee type other than
    `Bool` / `Any`, so `String` falls under that rule without
    special-casing.
  - Escape handling rides the lexer's existing
    `read_string` / `string_lit` decoders — a pattern literal
    `"a\n"` is lexed to the same two-byte string as an
    expression literal `"a\n"`, so runtime comparison is
    straightforward.
- The ticket's "commit message: RES-162: string-literal match
  patterns" is used on the test-adding commit so the board's
  ticket-id-in-subject invariant holds.
- Deviations: none.
- Unit tests (5 new, all covering documented AC bullets plus an
  edge case):
  - `string_literal_pattern_matches_exact_string` — success
    path across three arms
  - `string_literal_pattern_falls_through_to_wildcard` —
    ticket AC
  - `string_literal_pattern_decodes_escapes` — ticket AC;
    asserts `"a\n"` pattern matches a runtime string of `a + LF`
    and `"a\t"` falls into a different arm
  - `string_match_without_wildcard_is_non_exhaustive` — ticket
    AC for exhaustiveness
  - `string_literal_pattern_empty_string_matches` — regression
    for the `""` pattern (hand-rolled and logos lexers both
    produce `Token::StringLiteral("")` for it)
- Verification:
  - `cargo test --locked` — 440 passed (was 435 before RES-162)
  - `cargo test --locked --features logos-lexer` — 441 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
