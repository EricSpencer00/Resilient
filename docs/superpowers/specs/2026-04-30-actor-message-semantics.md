# Actor Message Semantics — V1 lock-in for V2 TLA+ preservation

**Date:** 2026-04-30
**Status:** Design recommendations / decision lock-in for #361
**Tracking:** RES-ACTOR-SEMANTICS (issue #361)
**Companion:** [2026-04-26-tla-model-checking.md](2026-04-26-tla-model-checking.md), [2026-04-30-tla-v2-design-lock-in.md](2026-04-30-tla-v2-design-lock-in.md)
**Unblocks:** [#124 RES-332](https://github.com/EricSpencer00/Resilient/issues/124) actor scaffolding implementation; downstream of [#270 RES-396](https://github.com/EricSpencer00/Resilient/issues/270) closure

---

## Why this document exists

V2's TLA+ encoding (per the
[2026-04-26 TLA+ companion spec](2026-04-26-tla-model-checking.md))
needs to know:

- Whether `send` is FIFO per (sender, receiver) pair, globally
  total, or arbitrary.
- Whether `receive` is atomic with the body that follows.
- The visibility of partial state during a `receive` body.
- What happens when an actor crashes mid-receive.
- Whether self-send is legal.

If [#124 RES-332](https://github.com/EricSpencer00/Resilient/issues/124)
ships its actor primitives without pinning these, V2's TLC
encoding has to retroactively pick — and pick wrong for half
the existing programs.

This document gives each of the five questions a recommendation,
the tradeoff analysis behind it, and a "fold back" line that
names the exact ticket whose acceptance criteria absorb the
answer. The recommendations are intentionally
**conservative** — make weak guarantees that are easy to
strengthen later, rather than strong guarantees that programs
become accidentally dependent on.

---

## Q1. Message ordering

### Question

> FIFO per (sender, receiver) pair? Globally total? No guarantee?

### Recommendation: **FIFO per (sender, receiver) pair**

Within a single (sender, receiver) pair, messages are delivered
in the order they were sent. Across different sender pairs,
ordering is **not** guaranteed — if `A` sends `m1` and `B` sends
`m2` to the same `C`, `C` may observe them in either order.

### Tradeoffs

| Option | Pro | Con | TLA+ encoding cost |
|---|---|---|---|
| No guarantee | Simplest implementation, matches CAN-bus / unreliable transports | Programs are extremely hard to reason about; even simple "increment then read" patterns become racy | Cheapest — every send/receive interleaving valid |
| FIFO per pair (recommended) | Matches Erlang / Akka / most actor frameworks; every "send X then send Y to same actor" pattern is intuitive; the receiver's ordering is locally deterministic | Cross-sender ordering is still nondeterministic, which surprises users who expected "global FIFO" | Modest — per-pair queues in TLA+, plus a nondeterministic merge into the receiver's mailbox |
| Globally total | Easiest to reason about | Forces a global ordering primitive (vector clock, central broker); kills concurrency on multi-core; absolutely impossible across distributed nodes | Expensive — global event log, every action constrains the order |

### Why FIFO-per-pair wins

Three reasons:

1. **Matches user mental model.** When you write `send a, x; send
   a, y;`, you expect `a` to see `x` then `y`. Programs that
   *don't* depend on this are rare; programs that *do* are
   common (logging, request/response, increment-and-read). A
   weaker guarantee would break ~every existing actor program
   in the wild.
2. **Sufficient for the verifier.** TLA+'s `Sequences` operator
   gives us per-pair FIFO trivially (`Append(mailbox, msg)`,
   `Head(mailbox)`); cross-pair nondeterminism is a single
   `\E sender \in OtherActors` choice per receive. Encoding cost
   is bounded.
3. **Strengthening is easier than weakening.** "FIFO per pair"
   can later be tightened to "FIFO globally" if a use case
   emerges; weakening from "FIFO globally" to "FIFO per pair"
   would silently break programs that learned to depend on the
   stronger guarantee.

### V2.0 acceptance criteria absorbed

- The `runtime.tla` module's `Mailbox` axiom (per the
  [TLA+ V2.0 lock-in spec, Q2](2026-04-30-tla-v2-design-lock-in.md))
  encodes per-pair FIFO. Specifically:
  `Mailbox = [pair \in (Sender × Receiver) |-> Seq(Message)]`,
  with `send(s, r, m) == Mailbox' = [Mailbox EXCEPT ![<<s,r>>] = Append(@, m)]`.
- [#124 RES-332](https://github.com/EricSpencer00/Resilient/issues/124)'s
  acceptance criteria add: "Two `send`s from the same actor to
  the same target deliver in `send` order; `send`s from different
  actors to the same target may interleave arbitrarily."
- Documented in `docs/concurrency.md` under the actor section.

---

## Q2. `receive` atomicity

### Question

> Is the message dequeue + handler body a single atomic step (TLA+
> `Next` action), or are intermediate states observable from other
> actors?

### Recommendation: **the receive handler body is atomic to other actors**

The dequeue + entire handler body run as one atomic TLA+
action — no other actor observes a partially-mutated state of
the receiver while a handler is mid-execution. This includes
nested `send`s from inside the handler (they all enqueue
atomically alongside the state mutation; observers see either
the pre-handler state or the post-handler state, never the
in-between).

### Tradeoffs

| Option | Pro | Con | TLA+ encoding cost |
|---|---|---|---|
| Atomic (recommended) | Matches Erlang single-receive semantics; trivial to reason about; `requires`/`ensures` on a receive handler are point-to-point invariants | Long-running handlers block the actor; users must keep handlers short or chain via additional sends | One TLA+ `Next` action per receive — minimal |
| Non-atomic (fine-grained interleaving) | Long handlers don't block the actor's mailbox; matches OS-thread mental model | State invariants become per-line, not per-handler; programs depend on TLC scheduler choices for correctness; every `send` from inside a handler can race the handler's own state read | Expensive — every statement inside a handler becomes a separate `Next` action |
| Atomic with explicit yields | Long handlers can split | Adds a `yield` keyword to the language; users have to know when to yield; the scheduler choice becomes a load-bearing design surface | Medium — explicit yield points become the only TLA+ action boundaries |

### Why atomic wins

The "atomic per handler" choice maps directly to
Erlang's `receive ... end`, which has 30+ years of production
evidence that it's the right default for safety-critical actor
code. Long-running handlers are an anti-pattern users learn
quickly; the runtime can emit a warning when a handler exceeds
a configurable wall-clock budget (V2.x follow-up — out of scope
for V2.0).

The non-atomic option is theoretically more concurrent but
defeats the verifier's value proposition: an `ensures`
clause on a handler asserts a postcondition over the handler's
*final* state. If the handler isn't atomic, that postcondition
is unenforceable (an interleaving observer might see a
mid-mutation state).

### V2.0 acceptance criteria absorbed

- The `runtime.tla` `Receive` action is structured as
  `Receive(actor, message) == /\ <dequeue> /\ <handler body translated as one big formula>`.
- [#124 RES-332](https://github.com/EricSpencer00/Resilient/issues/124)'s
  acceptance criteria add: "A receive handler executes
  atomically with respect to all other actors; its
  postcondition is the actor's observable state after the
  handler completes."
- Existing `verifier_actors.rs` already follows this convention
  (one obligation per receive handler, not per statement) —
  this PR ratifies the convention and binds V2 to it.

---

## Q3. Mailbox bounds

### Question

> Bounded queue with backpressure, or unbounded? What's the V2
> TLC encoding cost difference?

### Recommendation: **bounded with backpressure; default depth `8`; configurable per-actor up to `255`**

Each actor has a bounded mailbox. `send` to a full mailbox
**blocks the sender** (cooperative — the sender yields, the
runtime schedules the receiver, the sender resumes once space
is available). The default depth is `8` because:

- Most embedded use cases (the primary target — see CLAUDE.md
  goalposts) have shallow steady-state queue depths.
- 8 fits comfortably in the Cortex-M demo's 64 KiB `.text`
  budget.
- Powers-of-two simplify the runtime's circular-buffer
  allocator.

The depth is overridable per actor with `actor Counter[mailbox = 32] { ... }`
syntax up to `255` (a u8 fits the queue index — keeping the
runtime's per-actor overhead small).

### Tradeoffs

| Option | Pro | Con | TLC encoding cost |
|---|---|---|---|
| Unbounded | No backpressure mental model needed | Memory leaks on send-fast / receive-slow producer; for embedded targets this is a hard non-starter (no `alloc` in default features); TLC state space is unbounded ⇒ TLC fails | Infinite — TLC won't terminate |
| Bounded (recommended) | Bounded state space ⇒ TLC happy; backpressure forces the sender to handle the "full" case explicitly; matches embedded reality | Sender blocking propagates — a slow receiver can stall an upstream chain | Bounded by `mailbox_depth × n_pairs` — feasible for typical specs |
| Bounded with drop-on-full | No sender blocking | Silently dropping messages breaks every "I sent it, why didn't it arrive" debugging session; failure mode is invisible | Bounded — same as bounded-blocking |

### Why bounded-blocking wins

Two reasons stack:

1. **The default target is embedded.** Unbounded mailboxes
   require heap allocation; the default `resilient-runtime`
   feature set is `#![no_std]` with no allocator. Bounded is
   the only option that compiles.
2. **TLC needs a finite state space.** Unbounded mailboxes
   make TLC's enumerative search non-terminating. Even on the
   user-machine side (where heap is plentiful), V2's
   verification value evaporates if the model checker can't
   make progress.

The drop-on-full alternative is rejected because silent message
loss has been a reliability foot-gun in every actor framework
that's tried it (NATS pre-2019, early ZeroMQ). Backpressure is
explicit and debuggable.

### V2.0 acceptance criteria absorbed

- New runtime config: `actor Foo[mailbox = N] { … }` syntax;
  default `N = 8`, max `255`.
- Runtime: bounded circular buffer per actor; `send` blocks
  the sender (cooperative yield) when the target is full.
- TLA+ encoding: `Mailbox` axiom carries the bound;
  `send(s, r, m) == Len(Mailbox[<<s,r>>]) < bound /\ <append>`.
- `runtime.tla`'s `Mailbox` axiom in V2.0 hard-codes `bound = 8`
  for the V2.0 demo specs; per-actor overrides come in a V2.x
  follow-up that makes the bound a parameter of the spec.

---

## Q4. Failure visibility

### Question

> When an actor crashes mid-receive, is the message redelivered,
> dropped, or moved to a dead-letter mailbox?

### Recommendation: **dropped by default; configurable to dead-letter via supervisor strategy**

When an actor crashes mid-receive, the message that triggered
the crash is **dropped** — it does not get redelivered to a
restarted instance, nor does it linger in the mailbox. The
supervisor (per [#125 RES-333](https://github.com/EricSpencer00/Resilient/issues/125))
restarts the actor with a fresh empty mailbox; subsequent
`send`s succeed normally.

Dead-letter behaviour is opt-in via a per-actor supervisor
strategy: `supervisor Counter[strategy = "dead-letter"] { … }`
diverts crashed-message-handlers' inputs into a centrally
configurable `dead_letter_mailbox` that the user can drain.

### Tradeoffs

| Option | Pro | Con |
|---|---|---|
| Always redeliver | "At-least-once" semantics by default | A poison-pill message — one that always crashes the handler — kills the actor in an infinite loop. Real-world Erlang explicitly avoids this default. |
| Drop (recommended) | Poison pills can't trap the actor; `live { }` block recovery has clear semantics (the failure terminates the receive, the recovery runs once); matches the V1 `live { }` invariant #2 ("closed-form invariant — at most one recovery effect") from the [TLA+ spec](2026-04-26-tla-model-checking.md#v1-design-choices-this-spec-asks-us-to-preserve) | Users who want at-least-once semantics have to opt in (or build it themselves with explicit ack messages) |
| Dead-letter | Drop + observability — failed messages are inspectable | Adds a new global resource (the dead-letter mailbox); only useful if the user actually drains it; without the drain, it grows unbounded |

### Why drop-by-default wins

Three reasons:

1. **Poison-pill avoidance.** A message that crashes the handler
   re-delivered after restart will crash the handler again.
   Real-world actor frameworks have learned this the hard way
   (Akka before 2.4 had `at-least-once` by default; the v2.4
   release deprecated it).
2. **Composes with `live { }` blocks.** V1's `live { }` invariant
   says: at most one recovery effect per failure. Redelivery
   would force the "recovery" path to also include
   re-execution of the handler, which is a different effect.
   Drop keeps the two cleanly separated.
3. **Strengthening is easier than weakening.** "Drop by default"
   can be tightened with an opt-in dead-letter strategy
   (already part of the recommendation). "Redeliver by default"
   would break programs that learned to depend on the at-most-
   once delivery property.

### V2.0 acceptance criteria absorbed

- [#125 RES-333](https://github.com/EricSpencer00/Resilient/issues/125)'s
  supervisor strategies extend with a `"dead-letter"` option;
  default remains the existing `"restart"` (which now drops
  the in-flight message).
- The TLA+ `Crash(actor)` action sets the actor's mailbox to
  empty before restart; the in-flight message is removed from
  the model. The dead-letter variant is modeled as moving the
  message to a global `DeadLetters` set.
- `docs/concurrency.md` documents the default behaviour and the
  opt-in.

---

## Q5. Self-send

### Question

> Is sending to one's own pid legal? Does it bypass the queue?

### Recommendation: **legal, queues normally — does NOT bypass**

`send self, m` is well-formed and goes through the actor's
own mailbox — it does **not** invoke the handler synchronously
or bypass the queue. Self-send is just a regular send where
the receiver happens to be the same actor.

### Tradeoffs

| Option | Pro | Con |
|---|---|---|
| Illegal (compile error) | One less corner case in the TLA+ encoding | Self-send is genuinely useful — recursive workflows that schedule "next steps" for themselves; trampoline patterns that avoid stack growth |
| Bypass (synchronous) | "Free" — no mailbox traversal | Breaks atomicity (Q2) — the bypassed handler runs inside the *sender* handler's atomic step. Re-entrancy issues become legion. |
| Queues normally (recommended) | Self-send is just a send; no special case in the runtime, parser, typechecker, or verifier; atomicity is preserved | Slightly more memory churn for the trampoline pattern (one mailbox slot per recursion level) |

### Why "legal, queued" wins

The question is mostly about edge cases in the TLA+ encoding.
Every other answer creates a special case:

- "Illegal" requires a typechecker pass to detect `send self, …`
  and reject it; that's a parser-level walk just for one
  pattern.
- "Bypass" forces the runtime to special-case the receiver-id
  check, and breaks Q2's atomicity guarantee (the bypassed
  handler now runs inside the parent handler's atomic step).
- "Queues normally" matches the Mailbox axiom directly — `(self,
  self)` is just another `(sender, receiver)` pair, no special
  treatment needed.

The "free" performance argument for bypass is theoretical;
self-send is rare in practice (a few uses per program), and
the mailbox traversal is O(1) anyway.

### V2.0 acceptance criteria absorbed

- [#124 RES-332](https://github.com/EricSpencer00/Resilient/issues/124)'s
  acceptance criteria add: "`send self, m` is legal and behaves
  identically to `send other_actor, m` where `other_actor` happens
  to be `self`."
- The `runtime.tla` `Mailbox` axiom requires no change — the
  `(Sender × Receiver)` indexing already covers `(self, self)`.
- TLC encoding: nothing special.

---

## Sign-off summary

| # | Question | Recommendation | Risk if wrong |
|---|---|---|---|
| Q1 | Message ordering | FIFO per (sender, receiver) pair | Medium — strengthening to global FIFO is doable; weakening would break ~every program |
| Q2 | Receive atomicity | Atomic per handler (Erlang-style) | High — non-atomic would break every `requires`/`ensures` on a handler |
| Q3 | Mailbox bounds | Bounded with backpressure; default 8, max 255 | Low — the bound is a parameter, easy to retune |
| Q4 | Failure visibility | Drop by default; opt-in dead-letter | Medium — flipping to redeliver later would silently break drop-dependent programs |
| Q5 | Self-send | Legal, queues normally | Low — alternative answers are all worse, but the choice doesn't strongly bind V2 |

Q1, Q2, and Q4 are the load-bearing decisions; Q3 and Q5 are
implementation details that the V2 encoding will inherit
without much trouble either way.

---

## What this spec does NOT decide

- The exact syntax for `actor Foo[mailbox = N] { … }`. The form
  shown is illustrative; the parser PR for #124 picks the
  final shape.
- The wire format for the dead-letter mailbox or its drain API.
  V2.0 ships the strategy hook; the API lands in V2.x.
- Any behaviour for distributed actors (cross-process or
  cross-network). V2 is single-process.
- Whether `recv` (for `Channel<T>`-style synchronous
  communication, distinct from actor mailboxes) inherits the
  same semantics. That's a separate ticket — see the
  speculative sketch in `docs/concurrency.md`.

---

## Cross-references in `docs/concurrency.md`

The actor section of `docs/concurrency.md` should add:

> **Authoritative semantics:** see
> [docs/superpowers/specs/2026-04-30-actor-message-semantics.md](superpowers/specs/2026-04-30-actor-message-semantics.md)
> for the message-ordering, atomicity, mailbox-bounds, failure-
> visibility, and self-send rules. The actor system pinned by
> [#124 RES-332](https://github.com/EricSpencer00/Resilient/issues/124)
> implements those semantics; future changes go through that
> document, not through ad-hoc behavioural drift.

Updating `docs/concurrency.md` itself is folded into the
follow-up doc PR alongside this one, so both land together.
