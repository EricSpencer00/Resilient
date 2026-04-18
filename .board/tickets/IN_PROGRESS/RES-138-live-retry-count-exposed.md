---
id: RES-138
title: `live_retries()` builtin exposes retry count inside a live block
state: OPEN
priority: P3
goalpost: G10
created: 2026-04-17
owner: executor
---

## Summary
Live blocks (RES-036) retry on recoverable errors. Sometimes the
body wants to know which retry it is — e.g. to escalate a warning
after N retries, or to log. Expose a builtin that returns the
current retry count (0 on first attempt).

## Acceptance criteria
- Builtin `live_retries() -> Int`. Inside a live block returns the
  retry count (0..∞). Outside a live block is a runtime error
  `live_retries() called outside a live block`.
- Nested live blocks: the builtin returns the *innermost* block's
  retry count.
- Unit tests: inside single live block counts up correctly across
  forced failures; outside produces error with span.
- `examples/live_retry_log.rs` + `.expected.txt` prints
  `retry 0`, `retry 1`, `retry 2` during a three-failure-then-succeed
  sequence.
- Commit message: `RES-138: live_retries() builtin`.

## Notes
- Implementation: push retry counter onto a thread-local stack on
  live-block entry, pop on exit. The builtin reads the top.
- Don't also expose max-retry-limit — the user shouldn't be
  coupling control flow to the runtime's retry policy (which may
  change). RES-142 covers the declarative side.

## Log
- 2026-04-17 created by manager
