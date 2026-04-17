---
id: RES-142
title: `live ... within 10ms` wall-clock timeout clause
state: OPEN
priority: P3
goalpost: G10
created: 2026-04-17
owner: executor
---

## Summary
Retry forever is sometimes the wrong semantics — a control loop
would rather fail safely than re-spin forever. Add an optional
`within <duration>` clause that caps total time (retries
included) inside a live block. On expiry, the block escalates
exactly like exhaustion (RES-140).

## Acceptance criteria
- Syntax: `live within 10ms { ... }`, `live within 100us { ... }`.
  Duration literal is `<integer><unit>` where unit ∈ {`ns`, `us`,
  `ms`, `s`}. New `DurationLiteral` AST node.
- Runtime: take `Instant::now()` on block entry; before each retry,
  check elapsed vs budget; if over, treat as exhaustion.
- Interacts cleanly with `backoff(...)` (RES-139) — backoff sleeps
  count against the budget.
- no_std build uses the same clock abstraction placeholder as
  RES-139.
- Unit tests (std): tight inner body + 10ms budget exhausts; slack
  budget succeeds.
- Commit message: `RES-142: live within <duration> timeout clause`.

## Notes
- Duration literals are not a full time library — they only exist
  inside live clauses for now. Don't generalize.
- Combined syntax: `live backoff(...) within 50ms { ... }` — both
  clauses present in either order. Pin the order the parser
  expects and document.

## Log
- 2026-04-17 created by manager
