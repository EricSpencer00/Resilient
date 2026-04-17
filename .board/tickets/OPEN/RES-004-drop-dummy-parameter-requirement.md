---
id: RES-004
title: Drop dummy-parameter requirement (fn main() should parse)
state: OPEN
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

## Log
- 2026-04-16 created by session 0
