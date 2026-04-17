---
id: RES-136
title: Extract counterexamples from Z3 on verifier failure
state: DONE
priority: P2
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
"Could not prove `ensures x > 0`" is useful; "could not prove
`ensures x > 0` — counterexample: `a = -1, b = 0`" is dramatically
more useful. When Z3 returns `sat` on the negation, pull the model
and print variable bindings.

## Acceptance criteria
- When the verifier calls the solver with the negated goal and gets
  `sat`, extract the model via `z3::Model::eval` for each
  declared free variable.
- Format: `counterexample: a = -1, b = 0` appended to the existing
  verifier error diagnostic.
- Variables with no assignment in the model are omitted (Z3 is
  free to not assign them).
- Only primitive values printed — BV values as decimal, arrays as
  summary ("len=N, ...") for now.
- New test `verifier_emits_counterexample` on a program that
  obviously fails.
- Commit message: `RES-136: counterexamples from Z3 models`.

## Notes
- If we're in `verifier-bv` mode (RES-134), print the decoded
  signed integer value, not the raw bitvector.
- Counterexamples are not formally part of the proof; just user
  aid. They should never be cached (RES-135).

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/verifier_z3.rs`
  - Added `prove_with_certificate_and_counterexample` — a superset
    of `prove_with_certificate` that returns
    `(Option<bool>, Option<ProofCertificate>, Option<String>)`
    where the third slot is a formatted counterexample pulled from
    Z3's model when the negated formula is satisfiable (i.e. the
    `Some(false)` and `None` verdict cases).
  - Added `extract_counterexample` — walks the identifiers the
    translator could emit, calls `Model::eval` per-var with
    `model_completion=false` so Z3's unassigned vars are silently
    dropped per the ticket, converts to i64 via `as_i64`, filters
    out identifiers already pinned in `bindings` (echoing back
    input is uninformative), and joins as
    `name = value, name = value` in BTreeSet order.
  - Kept `prove_with_certificate` as a thin wrapper over the new
    entry point so existing callers outside the typechecker don't
    need to churn.
  - Five new unit tests (`verifier_emits_counterexample_for_*`,
    `verifier_omits_counterexample_for_tautology`,
    `counterexample_omits_bound_identifiers`,
    `counterexample_names_multiple_free_identifiers`) plus small
    `ident` / `int` / `infix` test helpers for readability.
- `resilient/src/typechecker.rs`
  - Extended `z3_prove_with_cert` to return the counterexample
    slot, and the non-z3 stub to `(None, None, None)`.
  - Updated both call sites (function-decl contracts and call-site
    `requires`) to capture the counterexample and, on a
    `Some(false)` verdict, append
    ` — counterexample: <model>` to the existing error message.

Acceptance criteria matrix:
- Counterexample extracted via `z3::Model::eval` per declared free
  variable — yes, in `extract_counterexample`.
- Format `a = -1, b = 0` appended to the verifier error — yes
  (manual test: `/tmp/x.rs:1:4: fn f: contract can never hold
  (statically false clause) — counterexample: x = -1`).
- Variables with no assignment omitted — yes, the `model.eval`
  return is `Option` and we drop `None`.
- Primitive-only printing — yes, `Int::as_i64` only; the `BV` /
  bit-vector mode from RES-134 (not yet landed) will need its own
  decoded branch when the Manager opens it.
- New test `verifier_emits_counterexample` on an obviously failing
  program — yes (`verifier_emits_counterexample_for_contradiction`
  + the manual integration check above).

Verification:
- `cargo build` → clean.
- `cargo build --features z3` → clean.
- `cargo test` → 248 unit + 13 integration pass.
- `cargo test --features z3` → 261 unit (+5 new RES-136 tests) + 14
  integration pass.
- `cargo clippy --tests -- -D warnings` → clean.
- `cargo clippy --features z3 --tests -- -D warnings` → clean.
- Manual end-to-end: a `requires x > 5 && x < 0` clause now prints
  `counterexample: x = -1` on typecheck failure.
