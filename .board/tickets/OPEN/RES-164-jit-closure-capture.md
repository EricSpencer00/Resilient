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
- 2026-04-17 claimed and bailed by executor (oversized, new JIT calling convention)

## Attempt 1 failed

Oversized: this ticket invents and plumbs a fn-ptr + env-ptr
calling convention in the Cranelift-backed JIT, which today has no
closure support at all (`grep FunctionLiteral src/jit_backend.rs`
returns 0 hits). Independent pieces:

1. **Free-variable analysis as a reusable helper.** The interpreter
   walks free variables in `apply_function` but it's coupled to
   `Environment`, not extractable as a pure AST walk. Factoring
   that out is its own small ticket.
2. **Closure-struct layout + runtime env allocator.** `{ fn_ptr,
   env_ptr }` needs a representation on the JIT's value stack and a
   C-ABI `resilient_closure_new(...) -> *mut ClosureEnv` hook so
   the compiled body can build the env at the literal site.
3. **Lowering `Node::FunctionLiteral` to emit env construction +
   the closure struct.**
4. **Indirect call path:** `bcx.call_indirect(sig, fn_ptr,
   &[env_ptr, arg0, ...])` with a matching prologue in the callee
   that unpacks captures from the env pointer.
5. **Four end-to-end tests** driving `--features jit` on a running
   binary.

Each of 1–4 is the size of a full iteration on its own; 5 depends
on all of them.

## Clarification needed

Manager, please split into:

- RES-164a: extract `free_vars(&Node) -> BTreeSet<String>` helper
  + unit tests. Usable from interpreter and JIT.
- RES-164b: define the closure-struct layout and the
  `resilient_closure_new` runtime hook.
- RES-164c: JIT lowering of `Node::FunctionLiteral` — emits env
  construction + the fn-ptr struct.
- RES-164d: JIT lowering of the indirect call path, plus callee-
  side prologue that reads captures out of the env.
- RES-164e: the four end-to-end tests.

No code changes landed — only the ticket state toggle and this
clarification note.
