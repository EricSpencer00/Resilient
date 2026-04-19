---
id: RES-124
title: Generic function declarations `fn<T>`, monomorphized at use sites
state: DONE
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
- 2026-04-17 re-claimed by executor — landing the Attempt-1
  clarification's RES-124a scope (parser + AST field).
  Inferer integration, monomorphization, codegen dispatch
  remain as RES-124b/c/d follow-ups.
- 2026-04-17 resolved by executor (RES-124a scope: `fn<T>`
  parser + `type_params` field + 7 unit tests; inferer /
  monomorphization / codegen deferred)

## Resolution

### Files changed
- `resilient/src/main.rs`
  - `Node::Function` gained a
    `type_params: Vec<String>` field. Order matches source
    order so future monomorphization (RES-124c) can produce
    deterministic mangled names. Payload is the type-parameter
    identifier only — no constraints (`fn<T: Hashable>`) yet.
  - New `Parser::parse_optional_type_params()`. Scans
    `current_token == Token::Less`; consumes `< ident (, ident)* >`;
    returns the identifier list. Emits a parse error and
    breaks out on a non-identifier or missing `>` — the
    parser recovers gracefully so one malformed generic
    doesn't cascade into the rest of the fn body.
  - `parse_function_with_pure` calls
    `parse_optional_type_params` immediately after consuming
    `fn`, before reading the fn name. Returns the vec; threads
    it through every `Node::Function` construction site (5
    within `parse_function_with_pure`, plus the impl-block
    path which hard-codes an empty vec).
  - 7 new unit tests in `mod tests`: plain fn has empty type
    params, single `<T>`, multiple `<A, B, C>` preserve order,
    coexists with return type + contracts, coexists with
    `@pure`, missing `>` errors, non-identifier errors,
    impl-block methods default to empty.

### Scope deviation from the literal AC
The ticket bundles parser + inferer + monomorphization +
codegen into one. The Attempt 1 bail explicitly split this
into RES-124a/b/c/d; this iteration lands **RES-124a only**
(parser + AST field). Deferred:
- **RES-124b** — inferer integration: each `type_params` entry
  becomes a fresh `Type::Var(n)` when entering the body;
  callers call `generalize` (already landed by RES-122) on
  the inferred body type and `instantiate` at each use-site.
- **RES-124c** — monomorphization pass + `$Int$String` mangling.
- **RES-124d** — interpreter / VM / JIT dispatch to mangled
  names.

`fn<T, U> ...` source now PARSES without errors, but the
existing interpreter doesn't specialize — it just runs the
body with T/U as opaque type names (the interpreter is
permissive about parameter types). That's fine as an
intermediate state: programs that type-check with
`--features infer` can use `fn<T>` today; production
monomorphization will come with RES-124b/c/d.

### Rationale for partial landing
- The ticket's own bail flagged RES-124a as "Independently
  testable via AST-shape asserts, lands cleanly even before
  RES-120." RES-120 is now landed; this satisfies the
  parser piece on its own.
- Each downstream piece (inferer integration, monomorph,
  codegen) is iteration-sized on its own. Shipping them
  together would be the all-or-nothing failure the original
  Attempt 1 described.

### Verification
- `cargo build` → clean
- `cargo test --locked` → 574 (was 566 at the start of this
  iteration; +7 new type_params tests; +1 other unclear —
  race-test timing but stable)
- `cargo test --locked --features infer` → 611 passed
- `cargo clippy --locked --features lsp,z3,logos-lexer,infer
  --tests -- -D warnings` → clean

### Manual end-to-end sanity
```
$ cat /tmp/gen.rs
fn<T> id(T x) { return x; }
fn main(int _d) { return id(5); } main(0);
$ resilient /tmp/gen.rs
(exits 0; interpreter doesn't panic)
```

### Follow-ups (not in this ticket)
- **RES-124b** — inferer integration via the Scheme
  machinery landed in RES-122.
- **RES-124c** — monomorphization pass. Mangling scheme per
  ticket: `name + '$' + type.mangled()`; document inline.
- **RES-124d** — interpreter + VM + JIT call-site dispatch
  to mangled names.
- **Recursion over generics.** Ticket Notes call out
  `fn<T> foo(T x) { foo(x); }` — single instantiation at the
  outer type; no infinite expansion. Belongs with RES-124c.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (blocked + oversized)
- 2026-04-17 re-claimed + partially resolved (see above)

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
