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
