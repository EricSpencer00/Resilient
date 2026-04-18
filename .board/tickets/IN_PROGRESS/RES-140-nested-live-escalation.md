---
id: RES-140
title: Nested live blocks: inner exhaustion escalates to outer
state: OPEN
priority: P3
goalpost: G10
created: 2026-04-17
owner: executor
---

## Summary
Pin the semantics for nested `live` blocks. Today the inner block
retries up to its limit; if it still fails the program halts. The
natural semantics for a resilience-focused language is that inner
exhaustion becomes a single recoverable error at the outer block,
which then retries its whole body.

## Acceptance criteria
- Nested `live { live { ... } }` produces one retry at the outer
  block per *full* exhaustion of the inner block.
- Inner and outer retry counters are independent;
  `live_retries()` (RES-138) returns the innermost counter.
- Escape when exhausted at the top level: same error as today,
  with a new trailing note listing the retry depth at each
  nesting level.
- SYNTAX.md "Live blocks" section extended with a short
  subsection on nesting semantics, including a worked example.
- Unit tests: two-level nesting with forced-failure inner block,
  asserting outer retry count increments as expected.
- Commit message: `RES-140: nested live blocks escalate on inner exhaustion`.

## Notes
- This isn't about magic — it's about giving users a composable
  resilience story. Document "live blocks compose; don't be
  surprised by retry × retry = a lot" loudly.
- Consider (but don't implement here): a whole-program retry
  budget to prevent catastrophic thrash. Track as a follow-up.

## Log
- 2026-04-17 created by manager
