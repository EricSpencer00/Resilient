---
layout: page
title: Actor lifecycle model
nav_order: 33
permalink: /actor-lifecycle-model/
---

# Actor lifecycle model

`resilient-runtime/tla/runtime_lifecycle.tla` extends the handwritten actor
scheduler model with explicit deregistration. The original `runtime.tla` and
its configuration remain separate checks. Run the lifecycle configuration to
include termination paths in the explored state graph.

## Runtime mapping

`resilient/src/actor_runtime.rs::deregister_actor` removes a live actor's mailbox,
its executable function state, and its runnable and blocked scheduler entries.
For an unknown PID it returns `NotLive` without changing those collections.
The model adds the following atomic action for a live actor:

| Rust effect | Model update |
|---|---|
| Remove the mailbox registry entry | Remove the PID from `actors` and empty its indexed mailbox slot |
| Remove all runnable queue entries | Filter the PID from `runnable`, preserving other actors' order |
| Remove the blocked entry | Remove the PID from `blocked` |
| Discard queued messages with the mailbox | Add their count to `droppedCount`, a model-only accounting counter |

The model's `droppedCount` now includes explicit termination as well as crash
losses. It is not a runtime metric. Failed deregistration is represented by
stuttering; there is no new error-state variable.

## Checked properties

The lifecycle configuration checks all original scheduler safety invariants,
plus:

- `InactiveActorsClean`: inactive actors retain no queued messages, runnable
  entries or blocked entries.
- `RunnableUnique`: each runnable PID occupies exactly one queue position.

`NoLostMessages` remains an accounting equation: accepted sends equal received,
discarded and still-queued messages. It does not establish eventual delivery.
The inherited restart and crash-isolation properties are also checked against
the extended transition relation.

The checked instance has three actor slots, two message values, mailbox depth
two, at most two accepted sends, one crash and a temporary restart limit of one.
Deregistration itself needs no event counter: with those bounds the existing
state space remains finite. These are exhaustive checks of a bounded model,
not a proof for arbitrary actor counts or message traffic.

## Running the check

With Java and the official TLC jar installed:

```bash
python3 resilient-runtime/tla/tests/check_lifecycle.py --jar /absolute/path/to/tla2tools.jar
```

The command first requires successful checking of the lifecycle model. It then
checks four temporary variants which respectively omit runnable cleanup,
blocked cleanup, mailbox cleanup and discard accounting. Each must produce an
invariant counterexample; parsing errors, missing tools and timeouts do not
count as successful detection. The temporary variants never overwrite the
checked-in model. These counterexamples also exercise deregistration from
runnable, blocked and queued-message states.

For a single check through the compiler's existing bridge:

```bash
rz tla check resilient-runtime/tla/runtime_lifecycle.tla
```

The neighboring `.cfg` selects `LifecycleSpec`. Retain the model revision,
configuration, TLC version and actual result when citing a run. No required CI
job is added by this model.

## Scope

The model abstracts cooperative scheduling between atomic transitions. It does
not include an actor's executable function registry, executing frames, timers,
hardware interrupts, OS-thread interleavings or application code. The Rust
function-state cleanup needs its own implementation tests; this model does not
prove it. There is no automatic extraction or refinement proof connecting the
Rust code to these actions.

The base model reuses a finite set of abstract identity slots after termination;
the runtime allocates monotonically increasing PIDs. This model does not prove
PID non-reuse or stale-handle behavior. The temporal specification adds no
fairness assumptions, so cleanup safety should not be presented as a liveness
or service-recovery guarantee.
