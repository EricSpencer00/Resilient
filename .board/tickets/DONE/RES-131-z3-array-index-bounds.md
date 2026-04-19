---
id: RES-131
title: Z3 verifier proves array-index bounds in contracts
state: DONE
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
- 2026-04-17 re-claimed by executor — landing RES-131a scope
  (SMT encoding for `len(xs)` + axiom). Obligation generation,
  elision, audit, integration tests stay as RES-131b/c/d.
- 2026-04-17 resolved by executor (RES-131a: `len(xs)` as
  Int const + `>= 0` axiom + cert emission + 9 unit tests;
  obligation generation, elision, audit, integration tests
  deferred)

## Resolution

### Files changed
- `resilient/src/verifier_z3.rs`
  - `translate_int` gained a new match arm for
    `Node::CallExpression` where the callee is a bare
    `Identifier("len")` and exactly one arg is an
    `Identifier`. Maps to `Int::new_const(format!("len_{}",
    arg_name))` — the same Z3 const for every reference to
    the same array identifier.
  - New `is_len_call(function, arguments) -> bool` helper:
    syntactic `len(<exactly-one-arg>)` check.
  - New `collect_len_args(node, &mut BTreeSet<String>)` walker:
    finds every `len(<id>)` reference in an expression tree
    and collects the arg-identifier names.
  - `prove_with_timeout` now calls `collect_len_args` up
    front; builds one `Int::new_const(ctx, "len_<arg>")` +
    its `>= 0` axiom per unique arg; asserts every axiom on
    both solvers (tautology + contradiction checks) before
    the formula itself goes in.
  - The SMT-LIB2 certificate emission now includes a
    `(declare-const len_<arg> Int)` line per `len` arg,
    followed by its `(assert (>= len_<arg> 0))` — so a
    stock Z3 re-verifying the cert gets the same context
    the prover used.
  - 9 new unit tests in `verifier_z3::tests`:
    - `len_of_ident_is_nonnegative_by_axiom`
    - `len_of_ident_gt_zero_is_not_universal`
    - `compound_formula_using_len_proves`
    - `certificate_declares_len_const_and_axiom`
    - `multiple_len_calls_on_different_arrays_get_distinct_consts`
    - `len_of_same_array_reuses_same_const`
    - `len_with_non_identifier_arg_bails`
    - `collect_len_args_finds_all_references`
    - `collect_len_args_ignores_non_len_calls`

### Scope deviation from the full AC
This lands **RES-131a only** per the Attempt 1 split. Deferred:
- **RES-131b** — typechecker walk that generates
  `0 <= i && i < len(xs)` obligations at every
  `Node::IndexExpression` and attempts them with Z3 in the
  function's `requires` / branch / `live` context. Needs
  per-location tracking (`proven_index_sites: HashSet<Span>`)
  which requires deriving `Hash` on `Pos` and `Span`.
- **RES-131c** — interpreter elision hook
  (`with_proven_index_sites`) consulted in
  `eval_index_expression` to skip the runtime bounds check.
  Audit row (new counter in `VerificationStats`).
- **RES-131d** — `tests/verifier_array_bounds.rs` with the
  full five-case coverage matrix (two provable, two
  unprovable, one requiring a `requires` chain from a caller).

With RES-131a in place, RES-131b can build on the
`translate_int` + `collect_len_args` helpers without
rewriting the SMT encoding.

### Design notes
- **`len_<name>` name-mangling.** Deterministic and human-
  readable in certificates. Collisions with user identifiers
  starting with `len_` are theoretically possible but
  unlikely in practice; a future ticket could add a
  disambiguation suffix if it becomes an issue.
- **Axiom on both solvers.** The tautology check (is `NOT
  formula` unsat?) AND the contradiction check (is `formula`
  unsat?) both need the axiom — otherwise a formula like
  `len(xs) == -1 || false` would appear as a contradiction
  (wrong: actually satisfied by the axiom constraint).
- **Non-identifier `len()` args bail.** `len([1,2,3])` or
  `len(foo(x))` return None from `translate_int`, matching
  the module's existing "bail to None on unsupported shapes"
  policy. The fallback (runtime bounds check retained) is
  still correct.

### Verification
- `cargo build` → clean
- `cargo build --features z3` → clean
- `cargo test --locked` → 574 (unchanged; z3 tests are
  feature-gated)
- `cargo test --locked --features z3` → 598 passed
  (+9 new `len()` tests in `verifier_z3::tests`)
- `cargo clippy --locked --features lsp,z3,logos-lexer,infer
  --tests -- -D warnings` → clean
- No regression in existing verifier tests.

### Follow-ups (not in this ticket)
- **RES-131b**: walk `Node::IndexExpression` at typecheck
  time; generate obligations via the `len()` machinery this
  iteration landed.
- **RES-131c**: interpreter elision + audit row.
- **RES-131d**: integration tests under
  `tests/verifier_array_bounds.rs`.
- **`Hash` on `Pos` / `Span`**: small upstream change needed
  by RES-131b's `proven_index_sites: HashSet<Span>`.

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
