---
id: RES-164
title: JIT: closure capture by value (RES-072 Phase K)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
RES-042 landed function values in the interpreter. RES-104 got the
JIT as far as let bindings. To JIT an expression like
`let add = fn(x) { return x + n; };` inside a function, we need
capture — a way to get `n` into the compiled body. Start with
capture-by-value; capture-by-mutable-reference is a separate,
harder ticket.

## Acceptance criteria
- Lowering detects `Node::FunctionLiteral` with a non-empty free
  set.
- Free-variable analysis computes the captures at the literal site
  (already a helper in the interpreter; reuse it).
- JIT emits a closure struct `{ fn_ptr: *const (), env: *mut Env }`
  at the literal; calls through the struct thunk.
- Captured values are copied into the env on closure construction.
- Call site becomes indirect: `bcx.call_indirect(sig, fn_ptr, &[env_ptr, arg0, ...])`.
- Unit tests: closure capturing a single Int, closure capturing
  two Ints, closure called through a variable, nested closure
  (inner captures outer's capture).
- Commit message: `RES-164: JIT closure capture by value (Phase K)`.

## Notes
- Leave capture-by-ref for a future ticket. Most benchmarks don't
  need mutation through a captured variable.
- Cranelift lacks a native "closure" concept; we manually compose
  the fn-pointer + env-pointer calling convention. Document the
  convention in a comment at the top of the new lowering code.

## Log
- 2026-04-17 created by manager
