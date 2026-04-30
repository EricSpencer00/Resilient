---
title: Live Block Semantics
layout: default
parent: Design Philosophy
nav_order: 4
---

# Live Block Semantics (RES-210)

This page is the formal specification of the `live { ... }` construct
that the tree-walking interpreter implements today. It complements
the informal worked examples in
[`SYNTAX.md`](../SYNTAX.md#live-blocks) by pinning down the exact
operational rules every implementation (tree walker, bytecode VM,
future JIT) MUST obey.

The language of this page:

- **MUST / MUST NOT** — required for conformance.
- **SHOULD** — a strong default that an implementation is allowed to
  diverge from only when a ticket-level ADR records the deviation.
- **MAY** — strictly permissive; not part of the contract.

Line references below point at `resilient/src/main.rs` on the branch
that landed RES-210.

## 1. Entry conditions

A `live` block begins executing the moment control reaches the
`live` keyword in source order. There is no other entry barrier:

- The block MUST be syntactically well-formed (`live` /
  `live backoff(...)` / `live within <duration>` / both clauses, in
  either order, each at most once).
- All free variables referenced in the body MUST already be bound in
  the enclosing environment. A live block does NOT create new
  bindings visible to its parent; `let` declarations inside the
  block are scoped to the block's body.
- On entry, the interpreter:
  1. Takes a **deep clone** of the current environment as
     `env_snapshot` (`main.rs` line ~6695). This is what each retry
     attempt restores from.
  2. Pushes a new retry counter (initial value `0`) onto the
     thread-local `LIVE_RETRY_STACK` so `live_retries()` inside the
     body reads `0` on the first attempt. An RAII guard
     (`LiveRetryGuard`, `main.rs` line ~6706) removes the counter on
     every exit path — success, exhaustion, timeout, or a Rust panic
     unwinding through the block.
  3. If the block has a `within <duration>` clause, samples
     `std::time::Instant::now()` as the wall-clock deadline anchor.

## 2. Retry loop

The body is evaluated inside a `loop { ... }` (`main.rs` line ~6715).
On each iteration:

1. The body runs once.
2. Invariants (if any) are re-checked (see §3).
3. If both succeed the block returns the body's final value and the
   loop exits.
4. If either the body or an invariant returns `Err`, control falls
   through to the retry arm.

The retry arm:

1. Increments the local `retry_count` by `1`.
2. Writes the new `retry_count` into the thread-local stack so the
   next body invocation sees the bumped counter from
   `live_retries()`.
3. Bumps the process-wide `LIVE_TOTAL_RETRIES` counter **only when a
   retry will actually happen** (i.e. `retry_count < MAX_RETRIES` and
   the budget has not been exceeded).
4. Checks the timeout budget (see §6).
5. If the retry cap has been hit OR the budget has been exceeded,
   escalates (see §4).
6. Otherwise, sleeps for the backoff delay (see §5) and then restores
   the environment from a **fresh deep clone** of `env_snapshot` (a
   fresh clone per retry; otherwise the first retry's mutations would
   leak into the second).

The retry cap is a compile-time constant: `MAX_RETRIES = 3`
(`main.rs` line ~6688). A plain `live { ... }` block therefore runs
its body up to **three times** in total: one original attempt plus
two retries. After the third failure the block propagates.

`retry_count` semantics:

| event                           | `retry_count` after event |
|---------------------------------|---------------------------|
| entering the block              | `0`                       |
| first body failure              | `1`                       |
| second body failure             | `2`                       |
| third body failure (exhaustion) | `3` (no retry fires)      |

`live_retries()` returns `retry_count` as observed *inside* the body,
which means it reads `0` on the first attempt, `1` on the second,
`2` on the third, and is never observed equal to `MAX_RETRIES`
because the block has already escalated by then.

## 3. Invariant re-check order

Invariants (RES-036) are clauses that MUST hold at the end of every
successful body evaluation. They are recorded on the `LiveBlock`
AST node as `invariants: Vec<Node>` (`main.rs` line ~991) and
evaluated in source order after the body's final expression.

Ordering on a single attempt:

1. Body runs.
2. If body returns `Err`, go to the retry arm — invariants are NOT
   checked for a failed body.
3. If body returns `Ok`, evaluate each invariant in source order.
   The first falsy invariant converts into an `Err` whose message is
   `"Invariant violation in live block: <pretty-printed clause>
   failed"` and the retry arm fires.

A failing invariant is **indistinguishable from a body-level
runtime error** from the retry arm's point of view — both increment
`retry_count`, both trigger a backoff sleep, both count against the
`within` budget, and both eventually exhaust the block with the
standard `"Live block failed after N attempts"` error (after
wrapping with the invariant-violation message as the cause).

Ordering across retries: on attempt `k+1`, the body runs first, and
invariants are re-checked only if the body on attempt `k+1`
succeeds. Invariants are therefore **never** evaluated against the
partial, rolled-back environment of a failed attempt.

## 4. Fault handler — max retries exceeded

When `retry_count >= MAX_RETRIES`:

1. The interpreter logs the exhaustion line at stderr.
2. `LIVE_TOTAL_EXHAUSTIONS` is incremented.
3. The block returns `Err(msg)` where `msg` has the shape:

   ```
   Live block failed after <N> attempts (retry depth: <D>): <cause>
   ```

   where:

   - `N` is `MAX_RETRIES` (always `3` today);
   - `D` is the nesting depth at escalation time — `LIVE_RETRY_STACK.len()`
     at the point of exhaustion, measured inclusively (a single,
     non-nested block escalates at `depth: 1`);
   - `<cause>` is the error message from the final failing attempt
     (either the body's error or the invariant-violation message).

The block does NOT catch or suppress the error. Callers observe the
`Err` exactly as if a non-live statement had raised it. In
particular, an outer `live` block wrapping this one counts the
escalation as **one** of its own retries — not three — and may go
on to retry its own body.

## 5. Backoff

When the block is written with a `backoff(...)` prefix, the retry
arm sleeps for `cfg.delay_ms(retry_count - 1)` milliseconds between
retries (`main.rs` line ~6836). The default `BackoffConfig`
policy (RES-139) is:

- `delay_ms(n) = min(base_ms * factor^n, max_ms)`
- `base_ms` default: `1` ms
- `factor` default: `2` (capped at `10` at parse time)
- `max_ms` default: `100` ms

The first retry (after the first failure) therefore sleeps
`base_ms`, the second `base_ms * factor`, and so on, capped at
`max_ms`. Backoff MUST NOT fire before the first attempt (a plain
success on attempt 0 is zero-wall-clock). Backoff MUST NOT fire
after the final escalation (there is no retry 4).

A plain `live { ... }` without a `backoff(...)` clause carries
`backoff: None` and retries with zero sleep — the historical
behaviour preserved for source compatibility.

## 6. Timeout

The `within <duration>` clause (RES-142) is a wall-clock deadline
anchored at block entry (`main.rs` line ~6712). Duration literals
are `<integer><unit>` where `unit ∈ {ns, us, ms, s}`; they exist
ONLY inside this clause.

On every retry, before the backoff sleep and before the retry-cap
check's "should we try again?" branch, the runtime computes the
elapsed time since the anchor. If `elapsed >= budget`, the block
escalates with the **timeout** prefix:

```
Live block timed out after <N> attempt(s) (retry depth: <D>): <cause>
```

A timeout counts as an exhaustion for bookkeeping: it bumps
`LIVE_TOTAL_EXHAUSTIONS` the same way a retry-cap hit does.

Backoff sleeps count against the budget. A `live backoff(...) within
50ms` block that has already spent 49 ms in backoff sleeps will fail
the deadline check on its next retry attempt even if the body's
execution time is trivial.

The `no_std` runtime's clock is a placeholder and does NOT enforce
`within` today — embedded targets ignore the clause until a real
monotonic clock lands. This is a known divergence noted in
[`SYNTAX.md`](../SYNTAX.md).

## 7. State roll-back contract

**Live blocks guarantee roll-back of regular `let` bindings only.**

On retry, the interpreter replaces `self.env` with a fresh deep
clone of the entry-time `env_snapshot`. That covers:

- All `let` and `let mut` bindings visible at block entry — their
  values revert to what they were at the moment the `live` keyword
  was reached.
- All in-memory mutations to those bindings inside the failed
  attempt — the mutated copies are dropped.

The roll-back contract EXPLICITLY EXCLUDES:

- **`static let` declarations.** `static let` values live in
  `self.statics`, which is shared across attempts by design
  (`main.rs` line ~6211). Users relying on the retry semantics for
  a counter or cache MUST use `static let`; users wanting roll-back
  MUST use ordinary `let`.
- **External side effects.** Writes to files, sockets, hardware
  registers, `println`-style stdout, memory-mapped peripherals, FFI
  calls with observable effects — none of these are rolled back by
  the retry loop. The user is responsible for compensating side
  effects (e.g. emitting an idempotent "start" message, scoping
  writes behind a `static let committed = false;` guard, or
  re-initialising hardware at the top of the block).
- **Observable interleavings with other threads.** A concurrent
  observer reading the interpreter's env mid-attempt (not possible
  in the tree walker today, but a constraint for the future VM)
  would see the pre-roll-back state.

This contract is intentional. Resilient is designed for embedded
and safety-critical code where a transparent roll-back of I/O is
either impossible (hardware) or prohibitively expensive
(transactional memory). The language prefers a minimal, predictable
guarantee plus explicit user-level compensation over a magic
roll-back that silently fails on the boundaries that matter.

## 8. Nested live blocks

Nesting is allowed and composes exactly as described in
[`SYNTAX.md` § Nesting](../SYNTAX.md#nesting-res-140):

- Each nesting level carries its own retry counter, its own env
  snapshot, and its own backoff/timeout state.
- `live_retries()` reads the **innermost** counter — the top of
  `LIVE_RETRY_STACK`.
- When an inner block exhausts or times out, its `Err` escalates up
  and the enclosing `live` block treats it as **one** body-level
  failure. If the outer block still has budget, it restores its OWN
  env snapshot and re-enters the inner block from scratch (including
  a fresh inner retry counter).
- Error messages accumulate a `(retry depth: D)` note per level, so
  the full chain serialises the history of where each exhaustion
  happened. The outermost block's error has `depth: 1`; its direct
  child has `depth: 2`; and so on.
- Retries multiply across levels. Two nested default `live` blocks
  run the inner body up to `3 × 3 = 9` times before the outer gives
  up. Users wiring real hardware into a nested structure SHOULD
  attach a `backoff(...)` clause at the inner level to avoid a
  quadratic storm.

## 9. `live_retries()` builtin

`live_retries() -> Int` (RES-138, registered in the builtin table at
`main.rs` line ~4536) reports the retry counter of the innermost
enclosing live block.

- The first attempt reads `0`.
- The Nth retry reads `N - 1 + 1 = N` after the increment (so the
  second attempt reads `1`, the third reads `2`).
- Calling `live_retries()` outside any live block returns the error
  `"live_retries() called outside a live block"` (caught by the RAII
  guard's empty-stack check).
- Passing any argument returns an arity error.
- After a `live` block exits (success OR exhaustion), the guard is
  dropped and a subsequent `live_retries()` call again errors with
  the outside-block message.
- In nested blocks, the builtin reads the INNERMOST block's counter
  — retries at outer levels are invisible to an inner body.

Any future VM / JIT implementation MUST preserve all five of the
above properties. The test suite in
`resilient/tests/live_block_spec.rs` covers them end-to-end.

## Conformance checklist

An alternative backend (bytecode VM, JIT, AOT) is RES-210-conformant
iff it reproduces, on the same source, all of the following against
the tree walker:

1. Same observable sequence of body invocations per attempt.
2. Same value of `live_retries()` at every call site.
3. Same exhaustion / timeout error message shape (prefix and
   `retry depth` note).
4. Same roll-back behaviour on `let` bindings (including arrays /
   structs).
5. No roll-back of `static let`.
6. No roll-back of external I/O.
7. Same `LIVE_TOTAL_RETRIES` / `LIVE_TOTAL_EXHAUSTIONS` increments
   across the whole program.

Point 7 is the diagnostic-quality counter contract; it's allowed to
use relaxed-ordering atomics.
