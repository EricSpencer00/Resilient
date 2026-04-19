---
id: RES-164
title: JIT: closure capture by value (RES-072 Phase K)
state: DONE
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
- 2026-04-17 claimed by executor — landing RES-164a scope (free_vars helper only)
- 2026-04-17 landed RES-164a (free_vars module); RES-164b..e deferred

## Resolution (RES-164a — free_vars helper only)

This landing covers only the **RES-164a** piece from the Attempt-1
clarification split: a reusable `free_vars(&Node) -> BTreeSet<String>`
helper with unit tests. The closure-struct layout (RES-164b), JIT
lowering (RES-164c/d), and end-to-end tests (RES-164e) remain
deferred.

### Files changed

- `resilient/src/free_vars.rs` (new, ~540 lines with tests)
  - `pub fn free_vars(&Node) -> BTreeSet<String>` — pure AST walk,
    no `Environment`, no `Value`, no stdlib lookup.
  - Shared recursive `walk` handles every `Node` variant. Binders
    are added to the in-scope set at the correct point in each
    construct: let's binder after the RHS walks, fn params and fn
    self-name before the body, `result` for ensures clauses,
    `for`-in loop variable for its body + invariants, and
    `Pattern::Identifier` names per-arm in `match`.
  - Top-level `Program` hoists fn / struct / type-alias / let
    names up front so forward reference works, matching the
    interpreter's hoisting pass.
  - `#[allow(dead_code)]` on the module — the helper is unwired
    today; RES-164c/d will call it from the JIT lowering.
- `resilient/src/main.rs`
  - Added `mod free_vars;` declaration.
- **Twenty** new unit tests cover:
  - Empty / literal-only programs return no free vars.
  - Bare identifier reference is free.
  - `let` binding shadows, but its RHS still sees outer scope.
  - `fn(x)` literal — parameters bind, outer names free.
  - Closure capturing one name, two names, nested closures
    (inner sees outer's capture).
  - `for`-in binder masks the loop variable inside body.
  - `match` binds `Pattern::Identifier` per arm; wildcard doesn't.
  - Named top-level fn can call itself without being free
    (regression guard for the interpreter's self-bind behaviour).
  - Assignment to an unbound name surfaces as free.
  - `if/else` scope isolation (lets in one branch don't leak).
  - Determinism check across equivalent ASTs with different leaf
    orderings.
  - Struct literal field values are walked.
  - `while`'s RES-132a `invariants` field is walked.
  - Bool-literal sanity guard against leaf misclassification.
  - End-to-end parse → drill → `free_vars` on a real source
    snippet (`fn make_adder(n) { let add = fn(x) { x + n }; ... }`).

### Verification

```
$ cargo build                                   # OK (8 warnings, baseline)
$ cargo build --features z3                     # OK
$ cargo build --features lsp,logos-lexer,infer  # OK
$ cargo build --features jit                    # OK
$ cargo test --locked
test result: ok. 611 passed; 0 failed; 0 ignored
(+ 58 integration tests, all OK)
$ cargo test free_vars
test result: ok. 20 passed; 0 failed
```

### What was intentionally NOT done

- **RES-164b** — no closure-struct layout, no
  `resilient_closure_new` runtime hook.
- **RES-164c** — no JIT lowering for `Node::FunctionLiteral`.
- **RES-164d** — no indirect-call lowering.
- **RES-164e** — no end-to-end `--features jit` tests that
  exercise a capturing closure.
- No changes to `apply_function` or the interpreter's existing
  `Rc<RefCell<Env>>`-based closure model. The helper is additive.

### Follow-ups the Manager should mint

- **RES-164b** — closure-struct layout + `resilient_closure_new`.
- **RES-164c** — JIT lowering of `Node::FunctionLiteral` consuming
  the free-vars set from RES-164a.
- **RES-164d** — indirect-call lowering + callee prologue that
  unpacks captures from the env pointer.
- **RES-164e** — the four end-to-end `--features jit` tests.

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
