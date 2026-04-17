---
id: RES-011
title: Support bare `return;` statements
state: OPEN
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

## Log
- 2026-04-16 created by manager
