---
id: RES-124
title: Generic function declarations `fn<T>`, monomorphized at use sites
state: OPEN
priority: P2
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
With RES-122 (let-poly) we can generalize inferred fns, but users
can't *write* explicit generics — `fn swap<A, B>(A a, B b) -> (B, A)`
doesn't parse. Add the syntax + a monomorphization pass that emits
one concrete version per (fn, type-tuple) pair seen at call sites.

## Acceptance criteria
- Parser: `fn<T, U> name(T x, U y) -> T { ... }` produces a
  `Node::Function` with a `type_params: Vec<TypeParam>` field.
- Inferer: generics become fresh `Type::Var` when entering the
  function body; callers instantiate them freshly.
- Monomorphization pass: walk the AST post-inference, collect
  `(fn_name, type_args)` tuples, clone the AST per unique tuple
  renaming the fn to e.g. `swap$Int$String`.
- Codegen (interpreter + VM + JIT): dispatch calls to the
  monomorphized name.
- Unit tests: monomorphization of `id<T>` at Int and String
  produces two fns; the JIT can compile both.
- Commit message: `RES-124: generic fn<T> declarations + monomorphization`.

## Notes
- Mangling: `name + '$' + type.mangled()` — `$Int`, `$String`,
  `$Point` for structs, `$Array$Int` for `Array<Int>` etc. Document
  the scheme inline.
- Recursion over generics: `fn<T> foo(T x) { foo(x); }` instantiates
  once at the outer type; no infinite expansion.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (blocked + oversized)

## Attempt 1 failed

Blocked on the inference walker (RES-120, OPEN with
`## Clarification needed`) and top-level let-polymorphism
(RES-122, also OPEN with `## Clarification needed`). Two acceptance
criteria fail open without them: "Inferer: generics become fresh
`Type::Var` when entering the function body" and "Codegen
(interpreter + VM + JIT): dispatch calls to the monomorphized
name" — the latter assumes a monomorphization pass backed by
inference.

Even ignoring the deps, this is a multi-iteration ticket on its own:
parser syntax for `fn<T>`, inferer instantiation, monomorphization
pass with a mangling scheme, recursion-over-generics handling, and
three codegen backends (interpreter + VM + JIT).

## Clarification needed

Re-open once RES-120 and RES-122 land. Additionally, the Manager
should consider splitting this ticket into:

- RES-124a: parser support for `fn<T>` syntax + `type_params: Vec<...>`
  on `Node::Function`. Independently testable via AST-shape asserts,
  lands cleanly even before RES-120.
- RES-124b: inferer integration — fresh `Type::Var` at function
  entry; instantiation at call sites.
- RES-124c: monomorphization pass + mangling scheme.
- RES-124d: codegen dispatch (interpreter + VM + JIT).

No code changes landed — only the ticket state toggle and this
clarification note.
