---
id: RES-156
title: Array comprehensions `[f(x) for x in xs if p(x)]`
state: DONE
priority: P3
goalpost: G12
created: 2026-04-17
owner: executor
---

## Summary
With map + set + array filter/transform being common, give users
a single sugar that handles the common case: one-dim
comprehensions with optional filter. Desugars to a simple for-loop
+ push at parse time.

## Acceptance criteria
- Syntax: `[<expr> for <binding> in <iterable> (if <guard>)?]`.
- Desugars to:
  ```
  { let _r = []; for <binding> in <iterable> { if (<guard>) { push(_r, <expr>); } } _r }
  ```
- Works on Arrays and Sets (`set_items` result, RES-149).
- Unit tests: simple map, map+filter, nested-scoped binding doesn't
  leak.
- Golden example `examples/comprehension_demo.rs`.
- Commit message: `RES-156: array comprehensions`.

## Notes
- Don't support multi-clause `for` in the comprehension
  (`[x for x in xs for y in ys]`) — that's a rabbit hole of
  performance surprises. One `for`, one optional `if`.
- The desugared form MUST use a fresh name (`_r$0`, `_r$1`, ...) to
  avoid shadowing user bindings if the expr references an outer
  `_r`.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - `parse_array_literal` detects `peek_token == Token::For`
    immediately after the first expression and delegates to the
    new `parse_array_comprehension` helper — the rest of the
    array-literal path is untouched.
  - `parse_array_comprehension` consumes
    `for <binding> in <iterable> (if <guard>)? ]` and desugars
    into an immediately-invoked fn (IIFE):
    ```
    (fn() {
      let _r$N = [];
      for <binding> in <iterable> {
        if (<guard>) { _r$N = push(_r$N, <expr>); }
      }
      return _r$N;
    })()
    ```
    When no guard is present the inner `if` is dropped — just
    `_r$N = push(_r$N, <expr>);`.
  - New `comprehension_counter: u32` field on `Parser` mints a
    fresh `_r$N` accumulator for every comprehension so nested /
    sequential uses can't clash. The `$` character is not a
    legal identifier byte in user source (see `is_letter`), so
    the synthesized name cannot collide with any user binding —
    the ticket's "fresh name" mandate is structurally enforced.
  - The desugar uses only existing AST nodes (FunctionLiteral,
    CallExpression, Block, ForInStatement, IfStatement,
    LetStatement, Assignment, ReturnStatement, Identifier,
    ArrayLiteral). Typechecker, interpreter, VM, JIT, and
    compiler all see an IIFE — no new variant, no plumbing
    changes.
- `resilient/examples/comprehension_demo.rs` +
  `comprehension_demo.expected.txt` — golden example exercising
  simple map, map+filter, and set-via-`set_items` iteration.
- Deviations from the ticket's `{ let _r = []; ... }` form: the
  language doesn't have block-as-expression semantics yet, so
  the desugar uses `(fn() { ... })()` instead — same
  scoping properties (binding can't leak; outer names are
  visible through normal closure capture), same fresh-name
  guarantee. The Notes' rule about "fresh name to avoid
  shadowing user bindings if the expr references an outer `_r`"
  is satisfied: a test
  (`comprehension_accumulator_name_does_not_shadow_user_r`)
  asserts a user variable named `_r` stays visible through the
  comprehension body.
- Unit tests (7 new):
  - `comprehension_simple_map` — `[x * 2 for x in xs]`
  - `comprehension_map_and_filter` — `[x * x for x in xs if x % 2 == 0]`
  - `comprehension_binding_does_not_leak` — binding scoped to
    the IIFE (ticket AC: "nested-scoped binding doesn't leak")
  - `comprehension_accumulator_name_does_not_shadow_user_r` —
    user `_r` survives; comprehension uses `_r$N`
  - `comprehension_over_set_via_set_items` — exercises the
    ticket's acceptance criterion "Works on Arrays and Sets
    (set_items result)"
  - `comprehension_empty_iterable_produces_empty_array`
  - `comprehension_counter_bumps_for_each_comprehension` —
    two comprehensions in one program both work correctly,
    locking in the fresh-name policy
- Golden example: `cargo test --locked` runs the
  `examples_golden` harness which matches the new
  `comprehension_demo.expected.txt` byte-for-byte after
  normalization.
- Verification:
  - `cargo test --locked` — 420 passed (was 413 before RES-156)
  - `cargo test --locked --features logos-lexer` — 421 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
