# Concurrency Design Note — RES-208

**Status:** Parse-scaffolding landed; scheduler not yet wired.
**Feature flag:** `concurrency-preview`
**Related tickets:** RES-191 (`@pure`), RES-192 (`@io`), RES-193 (effect polymorphism), RES-208 (this note)

---

## Actor Semantics

### State Ownership

Each actor owns its state exclusively. No reference to an actor's
internal state escapes the actor's body. This is the
data-race-freedom property stated positively: there is no shared
mutable state to race on.

In the preview syntax:

```
actor Counter {
    let mut count = 0;
    // ...
}
```

`count` is private to `Counter`. No other actor or top-level
function holds a reference to it.

### Message Typing

Messages are typed values: `Int`, `Bool`, `Float`, struct instances,
or arrays. The channel that connects two actors is typed at the
element level (a `Channel<Int>` only carries integers). This makes
message passing statically checkable once the scheduler lands; the
preview parser scaffolds the surface but does not enforce the type.

Provisional form:

```
send actorRef, 42;          // send integer 42 to actorRef
let n = recv replyChannel;  // receive next value from replyChannel
```

### `spawn` / `send` / `recv` Primitive Shapes

| Keyword | Form | Semantics |
|---|---|---|
| `actor` | `actor Name { body }` | Top-level actor declaration |
| `spawn` | `spawn Name` | Instantiates actor, returns opaque ref |
| `send` | `send target, message` | Enqueues message; non-blocking if capacity |
| `recv` | `recv channel` | Dequeues next message; cooperative block if empty |

---

## Supervisor Semantics

The inspiration is Erlang/OTP. A supervisor's job is to observe
child actors, decide what to do when one crashes, and take action.

### Restart Policies

Three restart strategies, borrowed from OTP:

| Strategy | Meaning |
|---|---|
| `one_for_one` | Restart only the crashed actor. Other actors continue unchanged. |
| `one_for_all` | Restart all children when one crashes. Useful when children share logical state. |
| `rest_for_one` | Restart the crashed actor and all actors started after it (source order). |

### Exit Propagation

When an actor exhausts its `live { }` retry budget (or encounters
an unhandled error), it "crashes" and sends an exit signal to its
supervisor. The supervisor applies its restart policy.

Composition rule: a `live { }` block inside an actor body is the
first retry level. If the live block exhausts its budget, it
propagates the failure up to the actor. If the actor cannot recover,
it propagates to its supervisor. The same vocabulary nests cleanly:
`live` → actor → supervisor.

---

## Scheduler Model

**Cooperative, not preemptive.** Actors yield at:

- A `recv` that finds an empty channel (cooperative block).
- An explicit `yield` (not yet in the grammar).
- Completion of the actor's current handler.

No Resilient code preempts other Resilient code. Preemption only
happens at the OS / RTOS layer. A Resilient actor running on a
FreeRTOS task will be preempted by the RTOS at RTOS-defined priority
boundaries, but it will not be preempted by another Resilient actor
unless it yields.

**Host RTOS relationship:**

```
   +--------------------------------------------+
   |  Host RTOS (FreeRTOS / Zephyr / bare loop) |
   |                                            |
   |   +-----------------------+                |
   |   |  One RTOS Task        |                |
   |   |  Resilient scheduler  |                |
   |   |  ┌────────┐ ┌───────┐ |                |
   |   |  │Actor A │ │Actor B│ |                |
   |   |  └────────┘ └───────┘ |                |
   |   +-----------------------+                |
   +--------------------------------------------+
```

The Resilient scheduler multiplexes actors inside a single RTOS
task. From the RTOS's perspective, there is one task. From
Resilient's perspective, there are N actors, each with a private
message queue.

---

## Effect-System Prerequisites

The soundness rule for concurrency: **only `@pure` functions may be
called from more than one actor concurrently**, with `send`/`recv`
as the only sanctioned `@io`-in-parallel primitive.

Required G18 tickets before runtime:

1. **RES-191** (`@pure` annotation) — purity predicate for the rule.
2. **RES-192** (`@io` inference) — infer the effect set of unannotated
   fns so the rule can be applied without user annotation burden.
3. **RES-193** (effect polymorphism) — HOFs like `map` inherit
   their callback's effect, so `map(@pure_fn, xs)` is pure.

Without these, the compiler cannot prove the shared-state-freedom
property. The preview parser lands independently; the runtime
blocker remains until RES-191/192/193 stabilise.

---

## Non-Goals for This Ticket

- **No preemption** of Resilient code by Resilient code.
- **No shared-memory concurrency.** Actors communicate only by
  message passing. No `Arc`, no `Mutex`, no `atomic`.
- **No unbounded work-stealing.** The scheduler is a simple
  round-robin over runnable actors in a single thread.
- **No `unsafe` atomics.** The surrounding C / RTOS layer owns
  anything that needs atomics.

---

## Open Questions (for follow-on tickets)

1. **Message types: structural vs nominal?** Should a channel be
   typed `Channel<Int>` (structural) or `Channel<IncrMsg>` where
   `IncrMsg` is a declared message type? Nominal is more auditable
   for safety-standard reviews; structural is lighter for small
   programs.
2. **Supervisor declaration syntax.** A `supervise { ... }` block
   was sketched in `docs/concurrency.md` but is not final. The
   ticket that mints the scheduler will finalize it.
3. **`spawn` return type.** Does `spawn Counter` return a typed
   `ActorRef<Counter>` or an opaque `ActorRef`? The typed form
   allows static channel-type checking; the opaque form is simpler
   to land first.
4. **Scheduling guarantees.** The first cut makes none. A follow-up
   can add deadline semantics once the WCET analysis path (AOT
   compilation) is in place.

---

## What Landed in RES-208

- `concurrency-preview` feature flag in `resilient/Cargo.toml`.
- Four new tokens: `Actor`, `Spawn`, `Send`, `Recv` (feature-gated).
- Four new AST nodes: `Node::ActorDecl`, `Node::Spawn`, `Node::Send`,
  `Node::Recv` (feature-gated).
- Parser functions for each construct.
- `typechecker.rs`: new arms emit `warning[W0001]` and return
  `Type::Void` so downstream phases are not broken.
- `compiler.rs`, `formatter.rs`, `free_vars.rs`: exhaustive match
  arms added under the feature flag.
- `eval` in `main.rs`: each node rejects execution with a clear
  "concurrency-preview: not yet executable (RES-208)" message.
- `tests/concurrency_preview_smoke.rs`: 6 integration tests
  (parse acceptance + runtime rejection) gated on the feature.
- `docs/concurrency.md`: "Preview flag" note added.
- This design note.
