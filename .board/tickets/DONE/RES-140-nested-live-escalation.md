---
id: RES-140
title: Nested live blocks: inner exhaustion escalates to outer
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Audit finding: the nesting semantics the ticket calls for are
ALREADY natural from the existing `eval_live_block` + the
RES-138 `LIVE_RETRY_STACK` infrastructure:

- Inner exhausts → inner returns
  `Err("Live block failed after 3 attempts: <cause>")`.
- Outer sees that `Err` as a recoverable failure, increments
  its own counter, and retries its whole body (which re-enters
  the inner block, whose guard pushes a fresh counter).
- Inner and outer `live_retries()` counters are independent —
  the RES-138 RAII guard stacks them, and the builtin reads
  the innermost top.

The ticket's ask reduces to:
1. A clearer "retry depth per level" footer on exhaustion.
2. Regression-pinning tests.
3. SYNTAX.md extension.

Files changed:
- `resilient/src/main.rs`
  - `eval_live_block` exhaustion error gains a `(retry depth:
    N)` footer where `N = LIVE_RETRY_STACK.len()` at the point
    of failure (self included). As errors escalate up the
    nesting chain, each outer level wraps the inner's error
    with its OWN "after 3 attempts (retry depth: K)" prefix,
    so the composed message encodes the retry depth at every
    level.
  - Two new unit tests:
    `nested_live_inner_exhaustion_counts_as_one_outer_retry`
    pins that inner exhaustion translates to exactly one outer
    retry (3 × 3 = 9 total inner invocations via a `static let`
    counter) AND that both `retry depth: 1` and `retry depth:
    2` appear in the final error.
    `nested_live_retries_reports_innermost_counter` confirms
    `live_retries()` inside an inner block reads 0 / 1 / 2
    per inner attempt, resets to 0 when the outer retries
    re-enters the inner block — producing the sequence
    [0,1,2,0,1,2,0,1,2] over the full 9-invocation run.
- `SYNTAX.md` — new `### Nesting (RES-140)` subsection under
  `## Live Blocks` explaining the composition rule, the retry
  multiplication caveat (with a pointer to RES-139's backoff),
  and a worked example showing the nested "retry depth: N"
  footer shape.

Deferred per the ticket's own notes: whole-program retry
budget (hint, not a hard requirement) — left for a follow-up.

Verification:
- `cargo build --locked` — clean.
- `cargo test --locked` — 315 unit (+2 new RES-140 tests) + 3
  dump-tokens + 12 examples-smoke + 1 golden pass.
- `cargo clippy --locked --tests -- -D warnings` — clean.
- Manual: the nested-always-fail program prints the composed
  footer `Live block failed after 3 attempts (retry depth: 1):
  Live block failed after 3 attempts (retry depth: 2):
  ASSERTION ERROR: inner` on the driver's error channel.
