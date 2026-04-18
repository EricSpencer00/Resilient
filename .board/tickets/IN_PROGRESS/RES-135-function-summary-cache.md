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
