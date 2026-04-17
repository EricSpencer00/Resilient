---
id: RES-157
title: Fixed-size array type `[Int; N]` for stack allocation
state: OPEN
priority: P2
goalpost: G12
created: 2026-04-17
owner: executor
---

## Summary
Heap-allocated `Array<T>` is great for host use but wrong on
embedded — we want predictable memory layout and no_std friendliness
without alloc. Add a fixed-size variant with compile-known length.

## Acceptance criteria
- Parser: type `[T; N]` where `N` is an integer literal. Value
  construction: `[1, 2, 3]` with explicit annotation `[Int; 3]`
  or via inference from annotation.
- Typechecker: length is part of the type; `[Int; 3]` and `[Int; 4]`
  don't unify.
- Runtime layout: backed by `Vec<T>` initially (no real memory
  win), but the interpreter asserts on out-of-bounds at compile
  time when possible. The no_std runtime gets a real stack-backed
  layout in a follow-up ticket (RES-178 track).
- Typechecker rejects assignment to an out-of-bounds constant index
  `a[10]` where `a: [Int; 3]`.
- Unit tests: constant OOB detected, runtime variable index OK.
- Commit message: `RES-157: fixed-size array type [T; N]`.

## Notes
- N as an expression (not just literal) is deferred — that opens
  const-generics which is a separate, bigger ticket.
- Interop: implicit widening from `[T; N]` to `Array<T>` is
  disallowed (nominal-style) — users call `to_dynamic(a)` if they
  want the heap form. Reasoning: surprise allocation is a footgun.

## Log
- 2026-04-17 created by manager
