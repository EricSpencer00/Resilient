---
id: RES-252
title: "Wire `@trusted` extern ensures into the typechecker + SMT verifier"
state: OPEN
priority: P2
goalpost: G14
created: 2026-04-20
owner: executor
---

## Summary

FFI Phase 1 Task 10 added `prove_with_axioms_and_timeout` to the Z3 verifier,
but the end-to-end plumbing that registers `@trusted` extern `ensures` clauses
into the typechecker's scope (so they propagate as SMT assumptions when
verifying callers) was explicitly left out of scope. A `#[ignore]` tripwire
test documents the gap:

```
// resilient/src/main.rs:17630
#[ignore = "end-to-end trusted-ensures integration requires typechecker.rs changes (out of scope for Task 10)"]
fn trusted_extern_ensures_propagates_as_smt_assumption()
```

The failing scenario: a Resilient function with `ensures result >= 0` that
calls `@trusted abs_val` (which has `ensures result >= 0`) cannot be verified
today because the typechecker doesn't feed the extern ensures as an SMT axiom.

## Acceptance criteria

- `typechecker.rs` registers the `ensures` clauses of `@trusted` extern
  declarations into its identifier/contract table.
- When verifying a caller's `ensures`, the verifier receives the extern
  ensures as axioms via `prove_with_axioms_and_timeout`.
- The `#[ignore]` tripwire test
  `trusted_extern_ensures_propagates_as_smt_assumption` passes with the
  `z3` + `ffi` features enabled.
- No existing tests regress.
- Commit message: `RES-252: wire @trusted extern ensures into typechecker + SMT context`.

## Notes

- Relevant code:
  - `resilient/src/main.rs` ~17620–17660 (ignored test + context)
  - `resilient/src/typechecker.rs` (contract table + verifier invocation)
  - `resilient/src/verifier_z3.rs` lines 105–168 (`prove_with_axioms_and_timeout`)
- The test is gated on `#[cfg(all(feature = "z3", feature = "ffi"))]`.
- Removing the `#[ignore]` attribute once the implementation lands is the
  merge signal.

## Log
- 2026-04-20 created by analyzer (tripwire test at main.rs:17630)
