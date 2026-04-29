# Concurrency Design Note (RES-208)

This design note formalizes Resilient's actor-based concurrency model.
It is the engineering-facing companion to `docs/concurrency.md` (the
user-facing manual). It records design decisions, prerequisite tickets,
and the soundness rules that downstream implementations must respect.

## Status

- **2026-04-28** — first version. Actors (RES-332) and supervisors
  (RES-333) have landed. This note retroactively documents the contract
  those implementations are expected to honor.

## Actor semantics

### State ownership

- Each actor owns an isolated state cell. The cell's type is the
  actor's `state` declaration; it is initialized once at `spawn` time
  and may be mutated only inside that actor's `receive` handlers.
- No shared mutable state across actors. References to mutable cells
  cannot cross `send` boundaries — the typechecker rejects payloads
  that contain interior mutability.

### Message typing

- Each `send(target, msg)` site has its `msg` type checked against the
  declared `messages:` set of `target`'s actor type. A message that
  isn't in the set is a compile error.
- Message values are deeply immutable. `Vec`/`Map`/`String` payloads
  are conceptually copied (the runtime may share by reference because
  the immutability guarantee makes it observably equivalent).

### `spawn` / `send` / `receive` shapes

```
spawn ActorType(initial_state)         // returns ActorRef
send actor_ref message_value           // fire-and-forget, ordered per-target
receive {
    M1(payload) => { ... },
    M2(payload) => { ... },
}
```

- `spawn` returns an `ActorRef<T>`. The reference is `Copy` and may be
  freely shared (it carries no mutability of the spawned actor).
- `send` is asynchronous and never blocks the sender. Mailboxes are
  bounded (configurable per-actor; default 256) and overflow is a
  declared `fails Overflow` effect on the calling fn.
- `receive` is the only place an actor's state can be mutated. The
  typechecker enforces single-handler-per-message-variant at the AST
  level.

## Supervisor semantics

### Restart policies

Inherited from OTP, with adjustments for embedded targets:

| Policy        | Meaning                                                  |
| ------------- | -------------------------------------------------------- |
| `one_for_one` | Only the failed child is restarted.                      |
| `one_for_all` | Any child failure triggers a restart of all children.    |
| `rest_for_one`| Failed child + all children started after it.            |

### Exit propagation

- An actor that returns from its main loop exits **normally**.
- A panic / unwrapped failure / dropped linear resource exits **abnormally**.
- Restart strategies (`permanent`, `transient`, `temporary`) decide
  whether normal vs. abnormal exits are restarted.

### Composition with `live { }`

- An actor's `receive` body may contain `live { }` retry blocks.
- `live` retries are local: they do not cross actor boundaries.
- A supervisor's max-restart-rate budget (`N` restarts per `T` seconds)
  protects against infinite-restart loops; exceeding it propagates the
  failure up to the supervisor's supervisor (or terminates the runtime
  if there is none).

## Scheduler model

- **Cooperative** — actors do not preempt each other. The scheduler
  switches at `receive` boundaries and at explicit `yield` calls.
- **One Resilient program → one host RTOS task.** On embedded targets
  (FreeRTOS, Zephyr, bare-metal), Resilient's actors are multiplexed
  inside a single host task. There is no Resilient-on-Resilient
  preemption.
- **Yield points** are well-defined: `receive`, `await` (future
  ticket), `yield`, and any blocking syscall.

## Effect-system prerequisites (G18)

The concurrency model depends on these G18 effect-system rows to argue
soundness:

- `@pure` (RES-191): purity of inner expressions. A pure fn cannot
  capture an `ActorRef` and cannot transitively `send`.
- `@io` (RES-192): an actor's `receive` body is in `@io` context.
- `@actor` (future row): tags fns that are only valid inside a
  `receive` body (e.g. `self()`, `current_supervisor()`). RES-208 does
  not introduce this row; it is a follow-up.
- `@random` (separate ticket): orthogonal to actors, but the scheduler
  must be allowed to use `@random` for queue ordering decisions.

## Soundness rules

1. **No shared mutable state.** Message payloads must be deeply
   immutable. Payloads containing `Cell`/`RefCell`/`Mutex` are a
   compile error.
2. **Capability-bounded send.** A fn may only `send` to actors whose
   message type matches the value being sent. The typechecker
   discharges this at the call site.
3. **Supervisor leak prevention.** A supervisor that goes out of
   scope without explicit shutdown is a compile error (linear
   resource discipline, RES-385).
4. **Mailbox overflow is a tracked effect.** A fn that `sends` to a
   bounded mailbox without handling `Overflow` must declare it in its
   `fails` set.

## Non-goals

- Preemption of Resilient code by Resilient code.
- Shared-memory concurrency / fearless data races à la Rust threads.
- Unbounded work-stealing schedulers.
- `unsafe` atomics / lock-free data structures.
- Distributed actors across machines (RES-390 covers a separate
  cluster-invariant verifier; physical distribution is out of scope
  here).

## References

- `docs/concurrency.md` — user-facing manual.
- `resilient/src/supervisor.rs` — RES-333 implementation.
- `resilient/src/verifier_actors.rs` — RES-388 actor temporal asserts.
- `resilient/src/cluster_verifier.rs` — RES-390 cluster invariants.
- OTP design principles, ch. 6 (supervisors).
