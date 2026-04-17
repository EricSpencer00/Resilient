---
id: RES-131
title: Z3 verifier proves array-index bounds in contracts
state: OPEN
priority: P2
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
RES-067 wired Z3; RES-068 elides runtime checks for fully-proven
fns. Neither touches array indexing. A function like
`fn head(Array<Int> xs) -> Int requires len(xs) > 0 { return xs[0]; }`
is provably safe but today still emits a runtime bounds check.
Teach the verifier to recognize `xs[i]` as generating a proof
obligation `0 <= i < len(xs)` and discharge it against the
precondition context.

## Acceptance criteria
- Verifier (SMT encoding side): `xs[i]` adds obligations
  `(>= i 0)` and `(< i (len xs))` where `len` is an uninterpreted
  function constrained by `>= 0`.
- Context: function `requires` predicates, enclosing branch
  conditions (already handled by RES-064), and `live`-block
  assumptions all flow in.
- If both obligations prove, RES-068's elision applies: no runtime
  bounds check at that site.
- `--audit` flag gains an "array bounds" row summarizing
  proven / unproven indexing sites per function.
- Unit + integration tests (tests/verifier_array_bounds.rs): two
  provable examples, two deliberately unprovable, one relying on
  a `requires` chain from a caller.
- Commit message: `RES-131: Z3 proves array-index bounds`.

## Notes
- `len(xs)` is a runtime-known value; model it as an uninterp fn
  `len :: Array -> Int` with axiom `>= 0`. We don't need to model
  the array contents for bounds proofs — just the length.
- If Z3 returns `unknown`, that's a failure to elide, not a
  verification failure — runtime check stays in.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (oversized for one iteration)

## Attempt 1 failed

The ticket bundles five pieces of independently-sized work into one:

1. SMT encoding — teach `translate_bool` to recognize
   `Node::FunctionCall { name: "len", args: [xs] }` as an Int
   expression, mint a per-`(len, xs-identifier)` Int constant, and
   emit the `>= 0` axiom. (Today the verifier bails to `None` on
   *any* function call in an expression — see the header comment of
   `src/verifier_z3.rs`.)
2. Obligation generation — walk every function body finding
   `Node::IndexExpression`, assemble `0 <= i && i < len(xs)`, and
   feed it to Z3 with the function's `requires` / branch conds /
   `live` assumptions as context.
3. Elision hook — each proven index site needs to be remembered
   per-location (no `NodeId` exists today; `Span` would work but
   currently doesn't derive `Hash`, so it would need a trivial
   extension of `span.rs`), threaded from typechecker to
   Interpreter, and consulted in `eval_index_expression` to skip
   the runtime bounds check.
4. Audit row — new counter in `VerificationStats` + render in
   `print_verification_audit`.
5. Tests — `tests/verifier_array_bounds.rs` with five cases
   (two provable, two unprovable, one requiring a `requires` chain
   from a caller).

Each of these is roughly the size of a full iteration on its own;
together they fit uncomfortably. The elision piece (3) also needs
the interpreter to expose a hook parallel to `with_proven_fns`,
which is non-trivial plumbing given the Interpreter carries the
proven set through the `eval` walk.

## Clarification needed

Manager, please consider splitting:

- RES-131a: SMT encoding for `len(xs)` + the `>= 0` axiom (item 1).
  Independently testable via a unit test that asserts
  `prove(len(xs) >= 0, ...) == Some(true)`.
- RES-131b: typechecker walk that generates and attempts the
  obligation per `IndexExpression`, producing a `proven_index_sites:
  HashSet<Span>`. Includes deriving `Hash` on `Pos` and `Span`.
- RES-131c: Interpreter elision hook — `with_proven_index_sites`,
  consulted in `eval_index_expression`. Audit row.
- RES-131d: `tests/verifier_array_bounds.rs` with the full five-case
  coverage matrix.

Landing these incrementally keeps each commit reviewable and each
test failure locatable. No code changes on this bail.
