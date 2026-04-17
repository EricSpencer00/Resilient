---
id: RES-008
title: Coerce primitives to string for concatenation with +
state: DONE
priority: P1
goalpost: G2
created: 2026-04-16
owner: executor
---

## Summary
`minimal.rs` contains `println("The answer is: " + result);` where
`result` is an `int`. Today `eval_infix_expression` returns
`Type mismatch: "The answer is: " + 42` because `+` is only defined
for like types. For a language meant to be *simple*, the usual
scripting behavior of coercing the other side to string when one side
is a string is the right call. This unblocks `minimal.rs` and is a
prerequisite for RES-006 (golden tests).

## Acceptance criteria
- `"x=" + 1` evaluates to `Value::String("x=1")`
- `"x=" + 3.14` evaluates to `Value::String("x=3.14")`
- `"on=" + true` evaluates to `Value::String("on=true")`
- `1 + "x"` (int on the left) also produces `"1x"`
- `1 + 2` still produces `Value::Int(3)` — pure-int arithmetic unchanged
- `cargo run -- examples/minimal.rs` prints a line containing
  `The answer is: 42`
- Unit test covers string+int, int+string, string+bool
- Integration test in `examples_smoke.rs` asserts `The answer is: 42`
  appears in `minimal.rs` output

## Notes
- Edit `Interpreter::eval_infix_expression` in `resilient/src/main.rs`.
- Coercion uses `Value::Display`, which already matches the format
  (`Int(42)` → `"42"`, `Float(3.14)` → `"3.14"`, `Bool(true)` → `"true"`).
- Do NOT introduce implicit coercions for other operators (`-`, `*`,
  `/`, comparisons) — only `+` with at least one string operand.

## Resolution
- New `stringify_for_concat(&Value) -> Option<String>` helper in
  `main.rs` returns the textual form for int/float/bool/string,
  `None` for functions/builtins/void/return.
- `eval_infix_expression` gains an early branch: for `+` with at
  least one string operand, both sides get stringified and
  concatenated into `Value::String`.
- 4 new unit tests: string+int, int+string, string+bool, int+int
  (regression for the unchanged pure-int path).
- Integration test for `minimal.rs` upgraded to full end-to-end
  assertion: looks for "The answer is: 42" and "Program completed.".

Verification:
```
$ cargo run -- examples/minimal.rs
Starting the program...
The answer is: 42
Program completed.
Program executed successfully
$ cargo test
15 unit + 2 integration, all passing
$ cargo clippy -- -D warnings
clean
```

## Log
- 2026-04-16 created by manager
