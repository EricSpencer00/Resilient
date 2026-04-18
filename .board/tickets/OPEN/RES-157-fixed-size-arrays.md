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
- 2026-04-17 claimed and bailed by executor (oversized; nominal rule collides with existing code)

## Attempt 1 failed

Three blockers that together push this outside one iteration:

1. **Parser needs multi-token type annotations in four places.** The
   type-annotation slot on `let`, fn parameters, fn return types,
   and struct fields currently consumes exactly one `Token::Identifier`
   and stores it as a `String`. Adding `[T; N]` means extracting a
   new `parse_type_annotation` helper that handles both the simple
   and the bracket form, and then swapping it in at all four call
   sites. That's manageable, but it's one ticket's worth on its own.

2. **`ArrayLiteral`'s inferred type conflicts with the nominal rule.**
   The ticket's note is explicit: "implicit widening from `[T; N]` to
   `Array<T>` is disallowed (nominal-style)." But
   `ArrayLiteral` in the typechecker currently returns `Type::Array`
   (dynamic-heap). Three mutually-exclusive choices, each with
   problems:

   - (a) Keep `ArrayLiteral: Type::Array`. Then `let a: [Int; 3] =
     [1, 2, 3];` cannot typecheck without contextual inference —
     the RHS's type doesn't carry a length, and the LHS wants
     `FixedArray(Int, 3)`.
   - (b) Change `ArrayLiteral` to infer `Type::FixedArray(elem,
     items.len())`. That matches the LHS but breaks every existing
     use of `[1, 2, 3]` followed by e.g. `push(xs, 4)` —
     `push` expects `Type::Array`, and the nominal rule forbids
     widening.
   - (c) Allow FixedArray → Array widening for back-compat. That
     directly contradicts the ticket's note about preventing
     surprise allocation.

   None of (a) / (b) / (c) is self-contained; each forces companion
   edits across the stdlib builtin signatures and existing tests.

3. **Constant OOB detection is fine on its own**, but is the
   smallest of the three pieces and can't land without 1 and 2.

## Clarification needed

Manager, please consider splitting:

- RES-157a: parser support for `[T; N]` as a type annotation at all
  four sites (let / fn params / fn return / struct fields), stored
  as a multi-token string. Typechecker ignores the length info
  initially — `Type::Array` still. Independently testable by
  asserting the parse round-trips to the expected annotation string.
- RES-157b: decide the `ArrayLiteral` inference policy. Either (i)
  keep `Type::Array` and add an assign-time length-check pass
  that reads the LHS annotation, or (ii) change `ArrayLiteral` to
  `FixedArray` and migrate `push` / `pop` / `slice` signatures to
  accept either (one-time typechecker signature refresh). This is
  the hard design decision and deserves its own ticket.
- RES-157c: `Type::FixedArray(elem, len)` variant + unification +
  constant OOB detection at `IndexExpression`, built on top of
  157b's policy decision.
- RES-157d: `to_dynamic` builtin (mentioned in the notes, not the
  acceptance criteria — could be its own small ticket or fold into
  157c).

No code changes landed — only the ticket state toggle and this
clarification note.
