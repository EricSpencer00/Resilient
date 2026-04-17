---
id: RES-011
title: Support bare `return;` statements
state: DONE
priority: P3
goalpost: G4
created: 2026-04-16
owner: executor
---

## Summary
`fn main() { return; }` panics in the parser:

```
panicked at src/main.rs:532:46:
called `Option::unwrap()` on a `None` value
```

`parse_return_statement` unconditionally calls
`self.parse_expression(0).unwrap()`, which fails for `return;`
because there's no expression to parse. Bare return is normal in
void functions and should parse into `ReturnStatement { value:
Node::Void (or similar) }`.

## Acceptance criteria
- `fn foo() { return; }` parses without error
- `fn foo() { return 42; }` continues to work
- At runtime, bare return in a void function still produces `Value::Void`
- Unit test covers both cases
- No new `.unwrap()` panics introduced; use `record_error` for misuse

## Notes
- Fix is at `resilient/src/main.rs:530-532` in `parse_return_statement`.
- Needs a variant of `Node` for void expression (`Node::VoidLiteral`?)
  or just store `Option<Box<Node>>` as the return value.

## Resolution
- `Node::ReturnStatement.value` is now `Option<Box<Node>>` instead of
  `Box<Node>`. `None` means bare `return;`.
- `parse_return_statement` now checks for `Semicolon` / `RightBrace` /
  `Eof` immediately after `return` and returns `value: None` in that
  case. Otherwise it tries `parse_expression(0)` and, crucially, no
  longer calls `.unwrap()` — on `None` it records an error instead.
- Interpreter: bare return evaluates to `Value::Return(Box::new(Void))`.
- Typechecker: bare return's type is `Type::Void`.
- Two new tests: `parser_accepts_bare_return` (asserts `value.is_none()`)
  and `parser_accepts_return_with_value` regression.

Verification:
```
$ cargo test
17 unit + 1 golden + 2 smoke, all passing
$ cargo clippy -- -D warnings
clean
```

## Log
- 2026-04-16 created by manager
