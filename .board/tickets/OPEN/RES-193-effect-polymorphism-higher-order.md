---
id: RES-193
title: Effect polymorphism for higher-order functions (best-effort)
state: OPEN
priority: P3
goalpost: G18
created: 2026-04-17
owner: executor
---

## Summary
A function like `map(f, xs)` has effects exactly equal to the
effects of `f`. Hard-coding `map` as IO is conservative and wrong
(it's pure when `f` is pure). Add a single effect variable to
higher-order signatures so the call site instantiates it.

## Acceptance criteria
- Sig form extension: `fn<T, U, e> map(fn(T) -e-> U, Array<T>) -e->
  Array<U>` — effects after `->` with `-e-` syntax binding a
  fresh effect variable.
- At call sites, the variable is unified with the actual argument's
  effect set.
- Rules: effect vars must appear at least once on each side of an
  arrow ("effects flow through"); otherwise typecheck error.
- Unit tests: `map(pure_fn, xs)` classified pure;
  `map(io_fn, xs)` classified IO.
- Parser: `-e->` and `->` both accepted; bare `->` means "unknown
  effect var" = fresh.
- Commit message: `RES-193: effect polymorphism for HOFs`.

## Notes
- Syntax is ugly; we accept that for the MVP. A follow-up can
  clean it up once real usage gives us data.
- Don't export effect vars into user-land generics — they're
  internal plumbing for now.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 claimed and bailed by executor (prereq chain
  unmet — see Attempt 1)

## Attempt 1 failed

Bailing: the entire infrastructure the AC hangs on hasn't landed.

The ticket's anchor example is:

```
fn<T, U, e> map(fn(T) -e-> U, Array<T>) -e-> Array<U>
```

Four pieces of syntax must parse for this to be a real signature,
and none of them do today:

1. **`fn<T, U, e>` generics** — blocked by RES-124 (bailed). The
   parser has no concept of type parameters on a fn decl. Adding
   `fn<...>` land requires generics plumbing through the typechecker,
   which itself needs RES-120's inference (also bailed).
2. **`fn(T) -> U` as a parameter type** — verified: `parse_function_parameters`
   (main.rs:1949) only accepts a bare `Identifier` for the type
   slot. There's no ticket for function-type parameters yet.
3. **`Array<T>` as a parameter type** — same constraint; MVP
   arrays today are untyped `Type::Array` (no element type). RES-055
   noted but not scheduled.
4. **`-e->` effect-variable syntax** — doesn't exist. Introducing
   it without the above three gives us a parser feature with no
   semantics to wire it to.

Even piece (4) alone — the `-e->` lexer/parser extension — can't
be tested in isolation because there's no example program that
could parse around it. The unit tests in the AC
(`map(pure_fn, xs)` classified pure) presuppose a `map` fn that
takes a fn parameter, which requires (2) and (3).

### Clarification needed

Manager, please re-sequence. Options:

1. **Wait for the generics chain.** RES-120 (HM inference) →
   RES-122 (let-polymorphism) → RES-124 (generic fn<T>) → then
   RES-193 can build on them. Today RES-120/122/124 are all
   bailed on their own deps (RES-119's Diagnostic scaffolding,
   NodeId).
2. **Split RES-193 into smaller pieces.** Suggested shape:
   - RES-193a: function types as parameters — `fn(T) -> U` in a
     type slot. Independent of generics; monomorphic. Useful
     even without polymorphism (lets users pass callbacks).
   - RES-193b: `Array<T>` generic element-type syntax in type
     slots. Depends on RES-055 spinup.
   - RES-193c: `-e->` effect-var + unification, on top of both.
3. **Rewrite to a narrower target.** Drop the `map(f, xs)`
   example; instead pick one specific built-in HOF (e.g. a
   hypothetical `pipeline(f, g)`) and hard-code its effect
   unification behaviour without a general mechanism. Less
   ambitious, lands today.

No code changes in this attempt — only the ticket state toggle +
this note. Committing the bail as a ticket-only move so `main` is
unchanged.
