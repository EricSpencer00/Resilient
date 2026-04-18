---
id: RES-193
title: Effect polymorphism for higher-order functions (best-effort)
state: IN_PROGRESS
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
