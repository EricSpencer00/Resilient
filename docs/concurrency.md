---
title: Concurrency and Real-Time Scheduling
parent: Design Philosophy
nav_order: 3
permalink: /concurrency
---

# Concurrency and Real-Time Scheduling
{: .no_toc }

What Resilient does and does not do about concurrent execution,
interrupts, and real-time scheduling — today, and where the
design is heading.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Current Model: Single-Threaded

Resilient programs run in a single thread. All three execution
backends — the tree-walking interpreter, the bytecode VM
(`--vm`), and the Cranelift JIT (`--jit`) — evaluate a program
on one OS (or bare-metal) thread from `main` to program exit.
There is no `spawn`, no `async`, no green threads, no parallel
`for`, no work-stealing runtime. A running Resilient program
occupies exactly one CPU, executes one statement at a time, and
observes a single total order over its own side effects.

The embedded runtime (`resilient-runtime/`, `#![no_std]`) is the
same story. The crate is a pure library: it exposes a `Value`
type and the arithmetic / comparison operators over it, and
that is all. It does not start threads, does not install
interrupt handlers, does not call into any scheduler, does not
even take ownership of the program's `main`. Whatever
surrounds it — a C `main()` on a bare-metal Cortex-M4F, a
FreeRTOS task, a Zephyr thread — is responsible for getting
cycles to it.

What this means for interrupt service routines:

- **Resilient code does not run inside an ISR.** A program
  compiled with `--jit` or interpreted at the host level cannot
  be installed as an interrupt handler. The host platform is
  not designed for it, and the JIT's compile-time assumptions
  (heap access, tracing, relocation tables) would violate an
  ISR context anyway.
- **On embedded targets, the surrounding C / RTOS layer owns
  ISRs.** The Resilient runtime library is called from task
  context only. If an ISR needs to wake a Resilient-hosted
  task, it does so through whatever primitive the RTOS
  provides (semaphore, queue, event flag) — Resilient sees
  only the task-side read of that primitive.
- **There is no `unsafe` block in the language, by design.**
  This means Resilient code cannot, today, dereference a
  volatile memory-mapped register. MMIO lives in the
  surrounding C layer; Resilient consumes already-cleaned
  values.

The single-threaded guarantee is strong and load-bearing for
the safety story. A reviewer can read a Resilient function
top-to-bottom and know that nothing else mutates its locals
or arguments during execution. No hidden concurrency, no
memory model to reason about.

## Fault Tolerance vs. Concurrency

Two features of Resilient can superficially look like
concurrency but are not.

**`live { ... }` blocks are fault tolerance, not concurrency.**
A live block executes its body on the calling thread, checks
its invariants, and on failure restores the block's local
environment to its last-known-good snapshot and retries the
body — with exponential backoff between attempts (RES-142
timeout clause, RES-141 telemetry counters). At no point are
two things running simultaneously. The block just runs the
same code again, sequentially, with clean state. See
[Memory Model — live-block snapshot semantics](memory-model)
for what is and is not captured in the snapshot.

Live blocks handle:

- a sensor returning a transient out-of-range reading
- a checksum failing on a packet
- a contract's `assert` tripping mid-computation

Live blocks do not handle:

- two tasks contending for the same mutable buffer
- an ISR preempting a task in the middle of a struct update
- lock-free coordination across cores

These are concurrency problems. Resilient does not have an
answer for them yet — see the roadmap below.

**Retries are not a concurrency primitive.** A live block's
retry loop is deterministic and serial. It does not spawn a
worker, does not time out against a wall clock in a separate
thread, does not deliver an interrupt to itself. The backoff
is a plain `nop` / busy loop at the caller's thread
granularity.

## Interoperability with RTOSes Today

The realistic deployment shape today is **Resilient as a
single-threaded task inside a host RTOS**. The host provides
the scheduler, the ISR machinery, the synchronization
primitives, and the memory map. The Resilient runtime is a
library that the task's code links against and calls like any
other library.

```
    +---------------------------------------------------+
    |  Host RTOS (FreeRTOS / Zephyr / bare-metal loop)  |
    |                                                   |
    |   +---------------+   +---------------+           |
    |   |  Task A (C)   |   |  Task B       |           |
    |   |  ISR handlers |   |  Resilient    |           |
    |   |  MMIO         |   |  runtime      |           |
    |   |  IPC primitives|  |  (library)    |           |
    |   +-------+-------+   +-------+-------+           |
    |           |                   |                   |
    |           +---- queue / ------+                   |
    |                mailbox / shared mem               |
    +---------------------------------------------------+
```

**ISR handling is the surrounding layer's responsibility.** The
ISR is a C function registered with the vector table at the
platform level. It may deposit a value into a ring buffer, set
a flag, or signal a semaphore. The Resilient task reads that
value / flag / semaphore through a plain function call when
the RTOS schedules it in.

**Data exchange between an ISR and a Resilient task** uses
whatever mechanism the C side provides. The common patterns:

- **Volatile shared memory.** The C ISR writes a
  `volatile uint32_t`; the Resilient task reads it through a
  small C shim wrapping `ldr` with a `volatile` marker. On the
  Resilient side, each read produces an `Int` value; once that
  value is in hand, it can't be torn by another context,
  because the runtime is single-threaded.
- **Lock-free single-producer / single-consumer queue.** The
  ISR is the producer, the Resilient task is the consumer.
  Because Resilient is single-threaded, the consumer side does
  not need internal synchronization — the task reads a whole
  element out of the queue and works with it in isolation.
- **RTOS primitive (semaphore / event flag / mailbox).** The
  Resilient task blocks on the primitive via an FFI call
  (not a Resilient-language feature yet — the shim is in C).
  Wake-ups are driven by the ISR posting to the primitive.

**ISR-safety by construction.** Because only one Resilient
execution happens at a time, there is no scenario in which an
ISR preempts Resilient mid-update to a Resilient-owned value.
A Resilient `Int`, `Bool`, `Float`, or struct field is never
observed in a half-written state by another Resilient
execution, because there is no other Resilient execution. The
only interference possible is between Resilient and the
surrounding C layer, which is governed by the C layer's
discipline (volatiles, atomics, critical sections) — not
Resilient's.

**What the Resilient side must still do carefully.** Reading a
multi-word value from shared memory (e.g., a 64-bit timestamp
on a 32-bit MCU) is not atomic at the instruction level. The
C shim, not the Resilient caller, is responsible for tearing
protection (disable-interrupts-around-read, or a lock-free
read pattern like Peterson's double-read). Once the shim
returns a scalar, Resilient treats it as a single value.

## Effect Tracking (G18 — Prerequisite for Safe Concurrency)

Goalpost G18 on the [roadmap](https://github.com/EricSpencer00/Resilient/blob/main/ROADMAP.md)
is **effect tracking**: annotating every function with the set
of effects it can perform. The alphabet today:

- `@pure` — no reads or writes outside locals and arguments;
  no I/O, no `live` retries, no randomness. Deterministic as a
  function of its inputs.
- `@io` — performs I/O (file, network, MMIO shim, `println`,
  anything observable outside the process).
- `@random` — reads a non-deterministic source.

G18 is **closed** as of 2026-04-29 — RES-191 (`@pure`
annotation), RES-192 (`@io` inference), RES-389 (declared
effects on fn signatures), and RES-385c (linear × effect
interaction) all landed. Higher-order effect polymorphism
(RES-193) remains an open follow-up; the V1 surface is
sound without it (HOFs default to `@io`).

What matters for this document is *why* we block concurrency
on effects.

**Data races require knowing which functions touch shared
state.** The moment two Resilient executions can run
concurrently — whether on two cores, two preemptible tasks on
one core, or two actors multiplexed by a cooperative scheduler
— the compiler needs a way to prove that no two of them mutate
the same location. The simplest sound rule is "only `@pure`
functions may be called from more than one place
concurrently," with explicit channel / mailbox operations as
the only sanctioned `@io`-in-parallel primitive. Without
effect annotations, the compiler has no way to check that rule.

**How this maps to safety-standard requirements.** The
effect system is not cosmetic; it directly discharges
obligations downstream reviewers would otherwise impose by
hand:

- **MISRA C rule 8.6** — objects and functions used in more
  than one translation unit must have exactly one external
  definition. The analog in a Resilient world with concurrent
  tasks is: any shared mutable state must be reachable only
  through a nominated owner. Effect annotations let the
  compiler refuse a program that closes over shared state
  without declaring it.
- **ISO 26262 ASIL-B and above — freedom from interference
  between software components.** The standard requires
  evidence that a fault in one component cannot corrupt
  another's state, timing, or control flow. An effect system
  plus message-passing between actors produces exactly that
  evidence as a compile-time property, not a testing
  artifact.
- **DO-178C objective A-7 / DO-330 — verification of coupling
  between software components.** Data coupling and control
  coupling are required to be enumerated; effects make both
  explicit at every call edge.

Effect tracking is therefore not a concurrency feature. It is
the foundation a concurrency feature can rest on without
becoming a new source of safety-case risk.

## Actor Primitives (RES-332)

Resilient V1 ships three actor builtins that implement cooperative
message-passing concurrency on top of the single-threaded interpreter.
They are the working implementation of the "Erlang-style actors" shape
described in the roadmap section below.

### `spawn(fn) -> ActorPid`

Allocates a fresh actor running `fn` and returns its opaque process
identifier. The actor is immediately runnable but does not execute until
the main script finishes — execution happens in the cooperative scheduler
that runs after the program's top-level statements complete.

```rust
fn worker() {
    let msg = receive();
    println(msg);
}

let pid = spawn(worker);   // returns ActorPid
```

### `send(pid, value)`

Enqueues `value` into `pid`'s bounded mailbox (default capacity: **8**
messages). Returns `Void` on success. Errors if `pid` is unknown or the
mailbox is full. On success, if the target actor was blocked waiting for a
message, it transitions back to the runnable queue.

```rust
send(pid, 42);             // enqueue 42 into pid's mailbox
```

### `receive() -> T`

Dequeues and returns the oldest message from the current actor's mailbox.
If the mailbox is empty, the calling actor is marked blocked and the
scheduler suspends it until a future `send` re-readies it. **`receive()`
is only valid inside an actor function body.** Calling it outside an actor
context is a runtime error.

```rust
fn handler() {
    let v = receive();     // blocks until a message arrives
    println(v);
}
```

### Cooperative scheduler

Actors run to their next yield point without preemption — code between
`receive()` calls is atomic with respect to other actors. The scheduler
runs all pending actors after the main script exits. Actors execute in
FIFO order; `send()` appends to the back of the runnable queue.

### Deadlock detection

If the scheduler's runnable queue is exhausted while actors remain blocked
on `receive()` with empty mailboxes, it emits:

```
error: deadlock detected; N actor(s) blocked on receive() with empty mailboxes; PIDs: [...]
```

### Example: ping-pong

```rust
fn ponger() {
    let msg = receive();
    println("pong");
}

fn pinger() {
    let ponger_pid = receive();   // receive the ponger's PID
    let go = receive();           // receive the go signal
    println("ping");
    send(ponger_pid, go);         // forward to ponger
}

let ponger_pid = spawn(ponger);
let pinger_pid = spawn(pinger);
send(pinger_pid, ponger_pid);     // give pinger ponger's address
send(pinger_pid, 1);              // fire the go signal
// output: ping\npong
```

## Roadmap for Structured Concurrency

The target shape is **actor-based, not shared-memory**, and
the inspiration is explicit: Erlang/OTP supervisor trees, as
noted in the [design philosophy](philosophy#1-resilience).
The language already commits to "let it crash" semantics at
the block level (the `live { }` block is a supervisor of
scope = one block); the concurrency roadmap extends the same
model to scope = one actor.

The broad shape being designed toward:

- **Each actor owns its state.** An actor is a Resilient
  function plus a private local heap. No other actor has a
  reference to that heap. This is the data-race-freedom
  property stated positively: there is no shared mutable
  state to race over.
- **Actors communicate by message passing.** A message is a
  value — an `Int`, a struct, an array — sent through a
  typed channel. The send operation is `@io`; the send is
  non-blocking if the channel has capacity. The receive
  operation is `@io` and may block (cooperatively).
  **RES-777 adds a structural constraint**: message payloads
  must be by-value (no reference types). Reference types
  (`&int`, `&[Region] T`) in actor state or handler parameters
  would allow aliasing across actor boundaries, which would
  enable data races despite the actor model's isolation
  properties. This constraint is enforced at compile time.
- **Each actor has its own supervisor.** The Erlang pattern:
  when an actor crashes, its supervisor observes the exit,
  decides on a restart policy, and either restarts, escalates
  to its own supervisor, or lets the subtree die. A `live { }`
  block composes with this — the block is one level of
  retry, the supervisor is the next level up.
- **The scheduler is cooperative.** Actors yield at
  well-defined points (receive, explicit yield, completion).
  No preemption of Resilient code by other Resilient code;
  preemption only happens at the OS / RTOS layer, where it is
  the host's responsibility.

A syntax sketch — **speculative, not committed, may change**:

```rust
// Speculative — no ticket has claimed this syntax yet.

actor Counter {
    let mut count: Int = 0;

    receive Increment {
        count = count + 1;
    }

    receive Read(reply: Channel<Int>) {
        send reply, count;
    }
}

fn main() {
    let c = spawn Counter;
    send c, Increment;
    send c, Increment;
    let answer_chan = channel<Int>();
    send c, Read(answer_chan);
    let n = recv answer_chan;
    println("count = " + n);
}
```

Things this sketch is deliberately noncommittal about:

- Whether `actor` is a top-level declaration or a block form.
- Whether message types are structurally typed, nominally
  typed, or defined with a dedicated `message` keyword.
- Whether `spawn` returns a typed handle, an opaque
  `ActorRef`, or a pair of sender / receiver.
- How supervisors are declared (likely a separate
  `supervise { ... }` block that lists child actors and
  restart strategies).
- Scheduling guarantees — the first cut will not make any.

The point of the sketch is to show the shape, not to promise
the shape. Expect changes once the real design work starts.

**Authoritative semantics**: when [#124 RES-332](https://github.com/EricSpencer00/Resilient/issues/124)
ships actor primitives, the message-ordering, atomicity,
mailbox-bound, failure-visibility, and self-send rules are
pinned by
[docs/superpowers/specs/2026-04-30-actor-message-semantics.md](superpowers/specs/2026-04-30-actor-message-semantics.md).
That sub-spec is the source of truth for the V2 TLA+
encoding; future changes go through that document, not
through ad-hoc behavioural drift.

**How live blocks compose with actors.** A live block inside
an actor retries local work. If the live block's budget is
exhausted, it raises to the actor. If the actor can't handle
it, the actor crashes. If the actor crashes, its supervisor
decides the next move. The same failure-handling vocabulary
(`live` → actor → supervisor) nests cleanly from expression to
system.

## Real-Time Scheduling Considerations

**Resilient does not currently make real-time scheduling
guarantees.** There is no guarantee on worst-case execution
time, no bound on retry-loop wall-clock duration, no hard
deadline mechanism. Programs are correct-in-functional-terms;
they are not yet correct-in-timing-terms.

Practical consequences:

- **The JIT introduces non-deterministic latency.** Cranelift
  compiles lazily on first call, allocates in the host heap,
  and may trigger guard-page handling or ICache invalidation.
  None of that is safe for hard real-time code where a
  missed deadline is a safety event. The JIT is fine for
  dev-loop iteration and for soft-RT paths where an
  occasional 10 ms stall is tolerable; it is not fine for
  control loops running at 1 kHz or above with hard
  deadlines.
- **The bytecode VM is more predictable** — no on-line
  compilation, a fixed dispatch loop, no heap growth during
  steady-state execution of pure-integer code — but it still
  uses the host allocator for `String` / `Array` / `Map` /
  `Set` operations. A program that stays inside
  `Int` / `Bool` / `Float` and stack-allocated structs will
  run with predictable per-op cost on the VM; a program that
  allocates, won't.
- **The tree-walking interpreter is the least suitable of
  the three for RT** — every expression evaluation allocates
  intermediate environments. Useful for dev, unsuitable for
  a flight control loop.

**Where the design is heading:**

- **AOT compilation.** A Cranelift-based AOT path (or an LLVM
  backend, depending on target support) would emit a single
  binary with no runtime compiler, no lazy codegen, no JIT
  heap. That is the precondition for meaningful WCET
  analysis — the binary the analyzer sees is the binary that
  runs.
- **WCET analysis integration.** Once AOT lands, the
  standard tools (aiT, Chronos, OTAWA) can operate on the
  produced binary. Resilient's contribution at the
  language level would be to keep loops bounded (either by
  a `while` condition the verifier can pin, or by a `for`
  over a statically-sized array) so that the analysis
  doesn't dead-end on an unbounded cycle.
- **Retry-budget timing.** Today a `live` block retries with
  exponential backoff up to a default budget. The retry
  itself is not wall-clock-bounded — a future RES ticket
  will add a `deadline` clause tied to a platform-provided
  monotonic clock, so the block fails deterministically if
  its retries take too long in real time.

**Practical guidance today:**

- **Hard real-time work stays in C.** Control loops, ISRs,
  any code that has to meet a deadline at μs granularity —
  C or hand-written assembly, supervised by the RTOS.
- **Soft real-time work can use the VM.** Deadlines in the
  10–100 ms range, algorithms with bounded-but-variable
  cost, code paths where an occasional slower iteration is
  acceptable.
- **Batch / one-shot analysis can use the JIT.** Startup
  self-test, offline telemetry post-processing, config
  validation.
- **ISRs never call into Resilient.** Full stop. If an ISR
  needs a computation done, it queues work for a
  Resilient-hosted task.

## What This Means for Safety Standards

The single-threaded model is, surprisingly, a feature from a
certification perspective. It removes a large class of
evidence obligations.

**DO-178C (airborne software) — coupling analysis.**
Multi-task data coupling and control coupling have to be
analyzed at the system integration level (objectives A-7.7
and A-7.8). A single-threaded Resilient task collapses the
intra-Resilient analysis to a trivial result: there are no
internal tasks, hence no internal data coupling or control
coupling to enumerate. The analysis reduces to the boundary
between the Resilient task and its neighbors — which is
identical to the boundary any C task of the same shape would
present. The tool qualification question
(DO-330) for Resilient itself is orthogonal and is addressed
by the Z3 certificate story (see
[philosophy — verifiability](philosophy#2-verifiability)).

**ISO 26262 (road vehicles) — freedom from interference.**
The standard (Part 6, clause 7.4.8) requires evidence that a
software element cannot cause another element to fail through
shared resources, timing, or execution. For two ASIL-separated
Resilient tasks to be argued free of interference, each is a
separate Resilient program in its own address space (OS
partition or MPU-enforced region), with a declared message-
passing interface. Within a single Resilient program, the
single-threaded execution model means that intra-program
freedom from interference is structural rather than evidential.
When multi-actor Resilient lands (see roadmap above), the
argument has to be refreshed to cover the actor boundary —
which is exactly why effect tracking (G18) is the prerequisite.

**IEC 61508 / IEC 62304 / DO-178C common theme —
determinism.** Certification-grade code is required to behave
deterministically as a function of its inputs and state. The
JIT's on-demand compilation violates this for the first call
to each function; the VM's allocator can violate it under
heap pressure. An AOT build is the standard-compliant
configuration. This document should be read as a promise that
the design is heading there, not a claim that any build of
Resilient today is qualification-ready.

## Summary

- Today: one thread, three backends; `spawn` / `send` / `receive()`
  actor primitives available in the interpreter (V1).
- ISR-safe at the boundary because Resilient never runs in an
  ISR and is never preempted mid-value by another Resilient
  execution.
- Actor scheduling is cooperative — yield points are `receive()`
  and actor completion; no preemption between actors.
- Bounded mailboxes (default 8) prevent unbounded queue growth;
  deadlock detection fires when all actors block with empty mailboxes.
- Live blocks are fault tolerance, not concurrency.
- Effect tracking (G18) is the named prerequisite for the full
  actor-safety argument; V1 actors are for interpreter use only.
- The roadmap points to Erlang-style actors + supervisor trees,
  built on top of effect tracking and an AOT path.
- Real-time scheduling guarantees are a future work item gated
  on AOT; today, hard-RT code stays in C.

If you are evaluating Resilient for a safety-critical project:
the honest pitch is "a single-threaded, verifiable,
self-healing task that your RTOS runs alongside C code."
Anything richer is roadmap.

## Further reading

- [Design philosophy](philosophy) — the three pillars, including
  why "let it crash" is the model.
- [Memory model](memory-model) — live-block snapshot /
  restore / retry details and value ownership across tiers.
- [no_std runtime](no-std) — the embedded build and its
  feature flags.
- [ROADMAP.md](https://github.com/EricSpencer00/Resilient/blob/main/ROADMAP.md)
  — G18 (effect tracking) and the full goalpost ladder.
- [GitHub Issues](https://github.com/EricSpencer00/Resilient/issues)
  — the ticket ledger for everything above.
