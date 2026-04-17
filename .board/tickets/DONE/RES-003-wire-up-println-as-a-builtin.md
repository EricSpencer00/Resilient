---
id: RES-003
title: Wire up println as a builtin
state: DONE
priority: P0
goalpost: G2
created: 2026-04-16
owner: executor
---

## Summary
Every example program calls `println(...)` but `println` is not
registered in the interpreter's environment. `cargo run -- examples/hello.rs`
currently fails with `Error: Identifier not found: println`. Without it,
no end-to-end example works, which makes the language unusable and
blocks integration tests.

## Acceptance criteria
- `cargo run -- examples/hello.rs` prints `Hello, Resilient world!` and exits 0
- `cargo run -- examples/minimal.rs` reaches completion without
  "Identifier not found: println" (it may still error for other
  reasons — fix those in a follow-up ticket if so)
- `println` accepts any single Resilient value and prints its string
  representation followed by `\n`; multi-arg variant is **not** in
  scope for this ticket
- String concatenation `"prefix: " + value` continues to work (this
  already works for strings + ints via the infix `+` operator)
- A unit test confirms the interpreter environment contains a
  `println` entry after `Interpreter::new()`

## Notes
- Look at `Interpreter::new()` and the `CallExpression` evaluation path
  in `resilient/src/main.rs`.
- The existing `Value` enum includes `Value::Void`; `println` returns
  `Value::Void`.
- Register `println` as a `Value::Builtin(fn(&[Value]) -> RResult<Value>)`
  or whatever existing pattern the interpreter uses for first-class
  functions. If none exists yet, add a minimal `Builtin` variant.

## Resolution
- Added a `Value::Builtin { name, func }` variant with `type BuiltinFn
  = fn(&[Value]) -> RResult<Value>` signature.
- `Interpreter::new()` now calls `register_builtins(&mut env)` which
  inserts `println` into the top-level environment.
- `apply_function` dispatches on the new variant.
- `builtin_println` accepts 0 or 1 argument; strings print without
  surrounding quotes, other values go through `Display`; returns `Void`.
- Value needed a manual `Debug` impl because `BuiltinFn` doesn't play
  with derive in a useful way; kept Debug output concise.
- Two new unit tests confirm `println` is registered and the
  too-many-args case errors predictably.
- Integration test `hello_rs_prints_greeting` now asserts on stdout.

Verification:
```
$ cargo run --quiet -- examples/hello.rs
Hello, Resilient world!
Program executed successfully
$ cargo test
running 10 tests ... test result: ok. 10 passed
running 2 tests ... test result: ok. 2 passed
```

`minimal.rs` still errors on `"msg" + int` (string+int concatenation);
that is a separate follow-up (ticket to be minted).

## Log
- 2026-04-16 created by session 0
