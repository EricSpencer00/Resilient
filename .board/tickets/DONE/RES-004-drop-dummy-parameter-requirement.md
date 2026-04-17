---
id: RES-004
title: Drop dummy-parameter requirement (fn main() should parse)
state: DONE
priority: P1
goalpost: G3
created: 2026-04-16
owner: executor
---

## Summary
Today every function must declare a parameter (`fn main(int dummy)`)
per SYNTAX.md — a parser-convenience hack, not a design choice. This
makes examples verbose and uninviting. We need `fn main() {}` and any
other parameter-less function to parse and run identically to its
dummy-param counterpart.

## Acceptance criteria
- `fn main() { println("hi"); } main();` parses and runs (with RES-003
  landed) — no dummy argument needed
- `fn add(int a, int b) {}` still works unchanged
- A unit test in the parser module asserts that `fn foo() {}` produces a
  `Function { name: "foo", parameters: [], body: Block(...) }`
- `examples/hello.rs`, `examples/minimal.rs` updated to drop `int dummy`
  and `main(0)` — now `main()` + `main();`
- SYNTAX.md updated: remove the "Functions must always have parameters"
  section, replace with a note that zero-parameter functions are fine
- README.md updated where it references the dummy-param requirement

## Notes
- Fix in `Parser::parse_function_parameters` in `resilient/src/main.rs`.
  Current behavior in `parse_function` at the "Expected '(' after
  function name" path is a RECOVERY path, not the common path — the
  actual parameter parser is what enforces the constraint.
- Do NOT touch `parser.rs` (dead code until G6).
- Do NOT delete `convert_functions.sh` in this ticket — separate
  cleanup.

## Resolution
Turned out the parser's `parse_function_parameters` already had a
`if self.current_token == Token::RightParen { return empty }` fast
path — the apparent "no-params not supported" constraint was a
secondary symptom of the RES-001 lexer bug that ate the `(`. With
RES-001 landed, no parser changes were needed.

- Added `parser_function_with_no_parameters` unit test that locks
  `fn main() { ... }` into a passing regression.
- Rewrote `examples/hello.rs`, `minimal.rs`, `comprehensive.rs`,
  `self_healing2.rs`, `sensor_example2.rs` to drop `int dummy` params
  and their `(0)` call sites via `sed` over the `fn NAME(int dummy)`
  and `NAME(0);` patterns.
- `SYNTAX.md`: removed the "Functions must always have parameters"
  section; replaced with a historical note.
- `README.md`: removed the "Utility Script" section pointing at a
  never-committed `convert_functions.sh`, and updated the Syntax
  Requirements bullets.

Verification:
```
$ cargo test
11 unit + 2 integration, all passing
$ cargo run -- examples/hello.rs
Hello, Resilient world!
```

## Log
- 2026-04-16 created by session 0
