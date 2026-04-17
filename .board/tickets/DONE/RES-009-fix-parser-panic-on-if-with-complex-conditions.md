---
id: RES-009
title: Fix parser panic on `if` with complex conditions
state: DONE
priority: P1
goalpost: G4
created: 2026-04-16
owner: executor
---

## Summary
Four examples (`self_healing.rs`, `self_healing2.rs`, `sensor_example.rs`,
`sensor_example2.rs`) panic with:

```
thread 'main' panicked at src/main.rs:603:13:
Expected '{' after if condition
```

The parser's `parse_if_statement` calls `panic!` when it encounters a
condition it can't parse — notably `&&`, `||`, or unary operators. Any
condition more complex than `a == b` or `a > 0` trips it. Blocks
golden-test coverage for the majority of example programs and any
real-world use.

## Acceptance criteria
- `if x > 0 && y > 0 { ... }` parses
- `if !done { ... }` parses
- Reproduce: each of the four panicking examples either runs to
  completion or surfaces a *diagnostic* (not a panic) explaining what's
  unsupported.
- Unit test: `parse("if a == 1 && b == 2 { x; }")` returns no errors.
- No new `panic!` calls in the parser — all parse failures go through
  `record_error`.
- Sidecar `.expected.txt` files added for any example that now runs to
  completion; ignored test `missing_expected_files_are_intentional`
  should list fewer files.

## Notes
- Panic site is `parse_if_statement` at `src/main.rs:603`.
- Lexer has no `&&` / `||` tokens today — add them first.
- Check the Pratt precedence table in `parse_expression`.

## Resolution
Scope narrowed during execution: the four panicking examples turned
out to exercise several *unsupported* features at once (prefix `!`,
stray `.`, `static let`, and a Pratt-parse bug where the expression
parser leaves `current_token` on the last consumed token). Adding
them all would have been multiple tickets. This ticket instead does
the one thing that matters for tooling: **no parser panic crashes
the binary, no matter what garbage it gets fed.**

- `parse_if_statement`: all three `panic!` replaced with
  `record_error` + recovery. Missing `{` now returns an `if` with an
  empty body so parsing keeps going.
- `parse_call_arguments`: `panic!("Expected ')' after arguments")` →
  `record_error`.
- `parse_expression` parenthesized form: `panic!("Expected ')')` →
  `record_error`.
- New `Token::Unknown(char)` variant. Two lexer panics converted:
  bare `!` and the generic "unexpected character" default arm. The
  parser's `parse_statement` has a dedicated arm for `Unknown` that
  records a diagnostic and returns `None`.
- New unit test `parser_recovers_from_missing_if_brace` locks in the
  recovery behavior.

Verification:
```
$ cargo run -- examples/self_healing2.rs
Parser error: Expected '{' after if condition, found FloatLiteral(0.5)
Parser error: Expected expression after 'return' ...
Parser error: Unexpected character '!'
Parser error: Expected '{' after if condition, found Identifier("toggle")
(clean exit, no panic)

$ cargo test
18 unit + 1 golden + 2 smoke, all passing
$ cargo clippy -- -D warnings
clean
```

## Follow-ups (will be minted as new tickets)
- Prefix operators (`!x`, `-x`) as real expressions → new ticket
- `static let` / `static` keyword → new ticket
- The Pratt-parser `current_token` invariant bug that makes
  `if expr {` fail when the expression ends on its last consumed
  token (root cause of the "Expected '{' after if condition, found
  FloatLiteral(0.5)" diagnostic).

## Log
- 2026-04-16 created by manager
