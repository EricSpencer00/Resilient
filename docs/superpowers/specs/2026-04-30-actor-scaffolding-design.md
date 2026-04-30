# Actor Scaffolding — Implementation Design

**Date:** 2026-04-30
**Status:** Design lock-in for [#124 RES-332](https://github.com/EricSpencer00/Resilient/issues/124)
**Tracking:** RES-332
**Companion:** [2026-04-30-actor-message-semantics.md](2026-04-30-actor-message-semantics.md) — semantics already locked

---

## Context

[#361 RES-ACTOR-SEMANTICS](https://github.com/EricSpencer00/Resilient/issues/361) pinned the semantic
contract (FIFO per pair, atomic receive, bounded mailbox,
drop-on-crash, legal self-send). [#124 RES-332](https://github.com/EricSpencer00/Resilient/issues/124)
is the implementation ticket: ship `spawn(fn() { ... })` /
`send(pid, value)` / `receive()` builtins.

The existing language already has `actor Counter { … receive M
{ … } }` declaration syntax and a working host-side scheduler
in `crate::supervisor` / `crate::verifier_actors`. This ticket
asks for a *closure-based* alternative — a way to describe
"spawn this anonymous function as an actor, send it a message,
get a result back" without the upfront actor-declaration
ceremony.

## Decomposition

This is multi-PR work. The recommended split:

### PR 1 — Mailbox + PID infrastructure

* `Value::ActorPid(usize)` — opaque actor handle, stable for
  the lifetime of the program.
* New thread-local `MAILBOX_REGISTRY` (HashMap<usize,
  VecDeque<Value>>) backing every actor's mailbox.
* New `SCHEDULER` thread-local tracking which actors are
  runnable, blocked, or done.
* No new builtins yet — this PR just lands the data model and
  unit-tests it in isolation.

### PR 2 — `spawn` / `send` / `receive` builtins

Per-builtin semantics, mapped to the existing #361 lock-in:

* `spawn(fn)` — registers `fn` as a new actor with a fresh
  PID, enqueues its initial frame on the scheduler, returns
  `ActorPid`. The actor's mailbox is bounded (default 8 from
  the spec); the bound is configurable via a future
  `spawn_bounded(fn, n)` follow-up.
* `send(pid, value)` — appends `value` to `pid`'s mailbox.
  Errors with "actor PID is not live" if the target has
  exited. Self-send is legal (Q5 from the semantics spec).
* `receive()` — dequeues from the calling actor's mailbox.
  When the mailbox is empty, the calling actor yields to the
  scheduler; the scheduler resumes it once `send` enqueues a
  message into it. The handler body runs atomically (Q2).

### PR 3 — Cooperative scheduler

Round-robin: the interpreter's outer loop dispatches one
"step" per runnable actor before yielding to the next. A step
runs from a yield point (entry, after `receive`, after
explicit `yield`) to the next yield point. Resilient code
between yield points runs without preemption (atomic per
handler).

### PR 4 — Deadlock detection

When the scheduler finds zero runnable actors but ≥1 blocked
actor with an empty mailbox, emit a runtime error pointing at
the deadlocked PIDs and their last-known yield position.

### PR 5 — Ping-pong golden test + docs

The issue's acceptance criterion: "Golden test: ping-pong
between two actors." Lands the `examples/actor_ping_pong.rz`
+ `.expected.txt` pair. Updates `docs/concurrency.md` with the
new builtin signatures.

## Total estimated effort

5 PRs × roughly 1–2 days each = 1–2 weeks of focused work.
Each PR is independently shippable; intermediate states keep
the language usable (PR 1 alone introduces the data model;
PR 2 makes spawn/send/receive callable; PR 3 makes them
useful for actual concurrent programs).

## Recommendation

Schedule the 5 PRs sequentially on a single agent's session.
Don't bundle PR 3+4 — the scheduler change is delicate enough
that having the simpler send/receive (without scheduling) in a
prior PR makes regressions easier to bisect.

The ticket stays open until PR 5 lands. The
[2026-04-30-actor-message-semantics](2026-04-30-actor-message-semantics.md)
spec is the authoritative behavior guide for every PR; any
deviation lands as a follow-up to that spec, not as a quiet
implementation choice.
