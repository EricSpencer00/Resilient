---
id: RES-009
title: Fix parser panic on `if` with complex conditions
state: OPEN
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

## Log
- 2026-04-16 created by manager
