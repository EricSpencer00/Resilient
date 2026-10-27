# Blocking timer delay progress

A timer's peripheral counter is not a clock for an arbitrary blocking delay.
A one-shot stops that counter at expiry. The former `timer_delay_ms` loop kept
polling it, so a longer delay could spin forever despite continuing driver
`tick` calls. Explicit stop/reset could freeze the same loop.

A pending delay now owns a separate remaining-duration countdown. Every tick
advances that countdown before checking the peripheral's run state. One-shot
expiry still happens once, and its own counter and callback retain their normal
semantics. Delay completion uses neither the peripheral counter nor a wrapping
timestamp. A `u32` millisecond duration fits in the `u64` countdown, and an
oversized elapsed tick saturates the remaining duration at zero.

## Reservation and cancellation

Only one delay can own an allocation. Another start or delay returns
`InvalidState` while the reservation is waiting or cancelled but unacknowledged.
Stop/reset mark it cancelled. The owning waiter's next poll acknowledges that
state, frees the reservation and returns `InvalidState`. Completion likewise
stops the timer and frees its reservation in the same registry operation.
There is no later cleanup step that could stop a newly started operation.

Release or registry reset invalidates the allocation. A pending ticket keeps
its opaque allocation identity, so it returns `InvalidId` and cannot change a
replacement timer. See [timer allocation identity](TIMER_IDENTITY_MODEL.md).

The countdown and reservation use the existing registry boundary, without new
unsafe code, heap allocation or atomic-64 operations. This work preserves the
existing deployment assumptions for registry access and interrupt handling.

## Runtime checks

The additive liveness regression drives the actual timer API on host threads,
with a bounded timeout and explicit release/join cleanup on failure. Unchanged
delay code fails the one-shot, stop and reset cases; periodic delay, zero delay
and release are passing controls. The fixed tests also check that one-shot
callbacks fire once, competing operations are rejected, and the reservation
is usable after cancellation acknowledgement.

Four new local-state tests check post-expiry progress, cancellation ownership,
independence from a saturated peripheral counter, and the maximum millisecond
duration using 1,000 maximum-width ticks.

```sh
cargo test --manifest-path resilient-runtime/Cargo.toml --test timer_delay_liveness
cargo test --manifest-path resilient-runtime/Cargo.toml --lib timer::
```

## Bounded progress model

`resilient-runtime/tla/timer_delay.tla` begins with one active one-shot delay,
period 1 and requested duration 3. It checks type bounds, no lost reservation,
no premature success, and eventual return after success or cancellation.
Progress explicitly assumes weak fairness for positive ticks, completion
polls and cancellation acknowledgement. Cancellation is optional; it is not
assumed to rescue a frozen wait.

The checker requires temporal counterexamples when expiry stops countdown
advancement or cancellation acknowledgement is disabled. Dropping a pending
reservation must instead violate the reservation invariant.

```sh
python3 resilient-runtime/tla/tests/check_timer_delay.py --jar /absolute/path/to/tla2tools.jar --log-dir /absolute/path/to/evidence
```

This is a handwritten bounded model, not a Rust refinement proof or a physical
latency bound. It does not prove interrupt safety, fairness of the concrete
scheduler, callback termination, deferred-callback cancellation, or a general
multi-waiter refinement relation. Runtime progress requires continuing positive
ticks and execution of the waiter; a blocked driver or unscheduled waiter does
not satisfy those assumptions.
