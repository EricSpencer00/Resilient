---
id: RES-135
title: Function-summary cache for the verifier
state: OPEN
priority: P3
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
Proving `requires`/`ensures` on a caller re-proves the callee's
contract every time. Cache `(fn_name, precondition) →
postcondition` after the first successful discharge and reuse it.
On a 200-fn program this drops verifier wall-time noticeably.

## Acceptance criteria
- New struct `VerifierCache` keyed by `(fn_name, canonicalized
  precondition SMT-LIB string)`.
- On a second encounter of the same key, skip Z3 and use the
  cached post.
- Invalidation: cache is rebuilt per compilation run (don't persist
  across runs yet — that's part of RES-195's manifest story).
- Instrumentation: `--audit` prints `verifier cache: N hits / M
  miss / K re-proves` at the end.
- Unit tests: two callers sharing a precondition produce one
  solve + one hit; distinct preconditions produce two solves.
- Commit message: `RES-135: verifier summary cache`.

## Notes
- "Canonicalized" means alpha-rename variables and sort commutative
  operands — without this, semantically-equal preconditions miss
  the cache. Simple approach: run the SMT-LIB through a
  deterministic pretty-printer.
- If the `z3` feature is off, this code just isn't compiled in
  (gate the whole module behind the feature).

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (canonicalization is
  its own ticket; cache without it misses trivially)

## Attempt 1 failed

Three pieces bundled:

1. **Canonicalizer**. The ticket is explicit — "alpha-rename
   variables and sort commutative operands … without this,
   semantically-equal preconditions miss the cache." Correct
   canonicalization walks the expression AST, folds commutative
   ops (`+`, `*`, `&&`, `||`, `==`, `!=`) into sorted
   sequences, alpha-renames bound identifiers to a stable form,
   then pretty-prints to a deterministic SMT-LIB string. That's
   a full expression-tree transformation in its own right,
   wanting its own test matrix.
2. **Cache plumbing**. New `VerifierCache` struct keyed by
   `(fn_name, canonical_pre_smt2)` → cached post. Feature-
   gated behind `z3`. Threaded through every
   `z3_prove_with_cert` call site to try-hit before dispatch.
3. **`--audit` instrumentation**. New counters
   (`cache_hits` / `cache_misses` / `cache_reproves`) on
   `VerificationStats`, rendered in `print_verification_audit`.

Without piece 1, the cache hits approximately zero (two
functions' `x > 0` precondition produce different SMT-LIB
strings because Z3's translator assigns fresh variable
names). The ticket's own notes acknowledge this.

## Clarification needed

Manager, please split:

- RES-135a: AST canonicalizer — stand-alone pass with per-op
  unit tests covering alpha-rename, commutative reorder,
  nested expressions. Reusable beyond the cache (e.g. RES-119's
  Diagnostic dedup if that lands).
- RES-135b: cache + integration + audit counters. Lands on top
  of 135a.

135a is the interesting work; 135b is an iteration of glue.

No code changes landed — only the ticket state toggle and this
clarification note.
