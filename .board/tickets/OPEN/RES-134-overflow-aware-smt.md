---
id: RES-134
title: Overflow-aware SMT encoding using Z3 bitvector theory for i64
state: OPEN
priority: P3
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
The current SMT encoding treats `Int` as mathematical ℤ. That's
unsound for safety-critical: `ensures result > 0` proven
mathematically can still fail at runtime because `x + 1` overflowed
to `i64::MIN`. Switch the `Int` encoding to Z3's `BitVec 64` theory
and reissue proofs.

## Acceptance criteria
- Feature flag `verifier-bv` (opt-in, off by default while we
  measure impact on solve times).
- With the flag on, Int → `BitVec 64`; all arithmetic uses
  signed bitvector ops (bvadd, bvsub, bvmul, bvsdiv, bvsrem).
- Comparison / equality: `bvslt`, `bvsle`, `=`.
- Proof-emission side: `--emit-certificate` also dumps bv form
  when the flag is on.
- Unit test `verifier_overflow_fails` shows a previously-proven
  fn now fails without `requires` bounding the input.
- RESULTS file at `benchmarks/verifier/RESULTS.md` compares
  solve times on the `cert_demo` fn across Int-theory vs BV-theory.
- Commit message: `RES-134: optional BitVec<64> encoding for Int`.

## Notes
- BV solving is slower than LIA; keep the flag opt-in. Users with
  ranges small enough to be Int-clean can skip it.
- When BV detects an overflow, the error message must clearly
  attribute it to overflow, not to the predicate: "proof fails
  due to possible overflow at <span>".

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (full SMT-theory swap +
  cert template + benchmark + overflow UX)

## Attempt 1 failed

Five pieces bundled:

1. **Feature flag**: new `verifier-bv` in `resilient/Cargo.toml`,
   cfg-gating the BV encoding in `src/verifier_z3.rs` while
   keeping the LIA path live for the default build.
2. **Full SMT encoding swap**: every integer operator in
   `translate_bool` / `translate_int` becomes a BV equivalent
   (`bvadd`, `bvsub`, `bvmul`, `bvsdiv`, `bvsrem`, `bvslt`,
   `bvsle`, `=`). Whole-file rewrite of the translator's output
   for the BV path.
3. **Certificate emission** (`--emit-certificate`): parallel
   BV-form dump so re-verification against stock Z3 works under
   both theories.
4. **Overflow-specific error UX**: when a BV proof fails because
   of overflow, the diagnostic needs `"proof fails due to
   possible overflow at <span>"` rather than the generic
   "could not prove …" — counterexample post-processing on top
   of RES-136.
5. **Benchmark**: new `benchmarks/verifier/RESULTS.md` comparing
   LIA-theory vs BV-theory solve times on `cert_demo`.

Plus the `verifier_overflow_fails` end-to-end test.

## Clarification needed

Manager, please split:

- RES-134a: `verifier-bv` feature flag + BV-encoded translator
  alongside the LIA one (both paths coexist). Default build
  still takes LIA. `verifier_overflow_fails` unit test lives
  here. Biggest slice, but self-contained.
- RES-134b: certificate-emission template for BV form.
- RES-134c: overflow-specific error attribution on
  counterexamples + `benchmarks/verifier/` RESULTS file +
  runner script.

No code changes landed — only the ticket state toggle and this
clarification note.
