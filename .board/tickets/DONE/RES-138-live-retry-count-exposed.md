---
id: RES-138
title: `live_retries()` builtin exposes retry count inside a live block
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/main.rs`
  - New `thread_local! { LIVE_RETRY_STACK: RefCell<Vec<usize>> }`
    — one entry per active `live { ... }` block, top = innermost
    (nested-block semantics per the ticket).
  - New `LiveRetryGuard` RAII: `enter()` pushes `0`; `Drop`
    pops. `set(count)` overwrites the top when the retry
    counter advances. The guard drops on every exit path from
    `eval_live_block` (success, max-retry-exhausted, panic), so
    the stack cannot leak across blocks.
  - `eval_live_block` enters the guard at block start and
    calls `LiveRetryGuard::set(retry_count)` immediately after
    each failure so the next attempt sees the updated count.
  - New `builtin_live_retries(args)`: zero-arg, reads the stack
    top → `Value::Int(n)`. Empty stack surfaces
    `live_retries() called outside a live block`. Registered in
    `BUILTINS` + the typechecker's builtin env as
    `Type::Function { params: [], return_type: Int }`.
  - Four new unit tests:
    - `live_retries_outside_live_block_errors`
    - `live_retries_wrong_arity_errors`
    - `live_retries_counts_up_across_failures` (drives a live
      block with a `static let fails_left = 2` twice-fail-then-
      succeed pattern; asserts the `seen` static collects
      `[0, 1, 2]`).
    - `live_retries_after_block_exit_errors` (confirms the RAII
      guard pops — a post-live `live_retries()` is again a
      runtime error).
- `resilient/examples/live_retry_log.rs` + `.expected.txt` (new):
  the ticket's canonical demo. Prints `retry 0`, `retry 1`,
  `retry 2`, then `succeeded with 42` during a two-failure-then-
  succeed sequence within the default `MAX_RETRIES=3` budget.
  (The ticket's wording is "three-failure-then-succeed"; under
  the current limit three failures exhausts the block, so the
  example succeeds on the third *attempt* after two failures —
  which is exactly the retry-counter progression the ticket
  asks to print.)

Deviation noted: the example runs three attempts (2 failures +
1 success), not four attempts with three failures — anything
with three failures would trip the MAX_RETRIES=3 cap in
`eval_live_block` and propagate the error. The output shape
(`retry 0`, `retry 1`, `retry 2`) matches the ticket verbatim.

Verification:
- `cargo build --locked` — clean.
- `cargo test --locked` — 306 unit (+4 new) + 3 dump-tokens +
  12 examples-smoke + 1 golden (includes the new
  `live_retry_log.rs`) pass.
- `cargo test --locked --features logos-lexer` — 307 unit pass.
- `cargo clippy --locked --tests -- -D warnings` — clean.
- Manual: `resilient examples/live_retry_log.rs 2>/dev/null`
  prints the expected four-line stdout; `let n = live_retries();
  println(n);` outside any live block prints the dedicated
  runtime-error diagnostic with a RES-117 caret underline.
