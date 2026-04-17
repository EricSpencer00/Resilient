---
id: RES-053
title: Typechecker rejects type mismatches at compile time
state: DONE
priority: P0
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
The typechecker in `typechecker.rs` has always been permissive —
visiting nodes without enforcing anything. This is the ticket that
makes it load-bearing. With RES-052's type annotations in place, we
can now reject:

- Binary operators on incompatible types: `1 + "x"` (before the
  RES-008 string-coercion case, or between two types that have no
  rule).
- Assignment to a typed `let` with a mismatching RHS: `let x: int = "hi";`.
- Return statement type mismatch against a function's declared
  return type.
- Calling a non-function value.

## Acceptance criteria
- `let x: int = "hi";` produces a compile-time type error
- `1 + true` produces a compile-time type error
- `fn f() -> int { return "hi"; }` produces a compile-time type error
- `42()` (call a non-function) produces a compile-time type error
- The existing 93 tests continue to pass — no regressions
- At least 4 new tests for the above cases
- `cargo run -- --typecheck bad.rs` exits non-zero on ill-typed input
- `cargo run -- --typecheck good.rs` exits zero on well-typed input

## Notes
- Type enum gets variants: Int, Float, String, Bool, Void, Result,
  Array, Struct(name), Function (already exists).
- The string-concat coercion (RES-008) must be modeled so `"x" + 1`
  still passes.
- The `?` operator returns the inner Ok type; for MVP we treat all
  Result values as Type::Result (not parameterized).
- A violation is returned as `Err(String)` from check_node and
  bubbles up to the CLI's --typecheck path.

## Log
- 2026-04-17 created and claimed
