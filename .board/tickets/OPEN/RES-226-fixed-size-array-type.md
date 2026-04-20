---
id: RES-226
title: Fixed-size array type `[T; N]` for embedded targets
state: OPEN
priority: P2
goalpost: G16
created: 2026-04-20
owner: executor
---

## Summary
Add a fixed-size array type `[T; N]` where `N` is a compile-time integer constant. Essential for embedded/no_std work where heap-allocated arrays are unavailable.

## Acceptance criteria
- Parser accepts `[T; N]` in type position: `let buf: [Int; 8] = [0; 8];` and `fn f(buf: [Int; 8]) -> Void`.
- Fill syntax `[expr; N]` creates a fixed-size array with `N` copies of `expr`.
- Type-checker enforces `N` is a non-negative integer literal.
- Indexing `buf[i]` bounds-checks at runtime; out-of-bounds returns a `Result` error (no panic).
- `len(buf)` returns `N` as a constant.
- `resilient-runtime` represents `[T; N]` without `Vec` (Rust array `[T; N]` under the hood via const generics).
- Cross-compile smoke test passes for `thumbv7em-none-eabihf`.
- Golden test: `fixed_array_demo.rs` / `fixed_array_demo.expected.txt`.
- Commit message: `RES-226: fixed-size array type [T; N] with stack-resident runtime repr`.

## Notes
- Unlike growable arrays (`Array<T>`), `[T; N]` may not be resized — `push`/`pop` are a type error.
- The verifier can treat array length as a known constant and discharge bounds-check obligations statically.
- Prerequisite for no-alloc buffer handling in the Cortex-M demo.

## Log
- 2026-04-20 created by manager
</content>
</invoke>