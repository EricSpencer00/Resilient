# Timer allocation identity

Timer handles identify one allocation, not just a timer-table slot. A released
handle or a handle retained across `reset_all` must not read, start, stop,
reset, release or change callbacks on a later allocation in the same slot.
`TimerId::index()` remains the diagnostic slot index; retain the returned
opaque handle when calling the timer API.

The runtime stores a generation with each slot and in the opaque handle.
Allocation advances it; release and registry reset preserve the slot's last
generation. Every handle entry point validates both the allocated state and
the generation. The handle uses eight index bits and 56 generation bits.
Generation zero is never issued. After generation `2^56 - 1`, that slot is
retired rather than wrapping; allocation returns `Exhausted` if no reusable
slot remains. No atomic-64 operation, heap allocation or new unsafe code is
needed: mutation uses the existing registry boundary.

The unresolved stale-handle finding was recorded in
[#4539](https://github.com/EricSpencer00/Resilient/issues/4539), then closed
because the bounded-tick work occupied the same source file. That tick work
merged separately; it did not add allocation identity. The implementation
and bounded model follow-up are tracked in
[#5096](https://github.com/EricSpencer00/Resilient/issues/5096).

## Checks

The new runtime regression file exercises every handle entry point after
slot reuse, observes the replacement's counters and callback delivery, repeats
registry reset/reallocation 64 times, and checks that an ordinary timer reset
preserves its live handle. The unchanged implementation fails the two stale
identity tests while the live-handle control passes. The generation and allocator tests also check the final generation, handle
index decoding, skipping a retired slot, and whole-registry exhaustion.

```sh
cargo test --manifest-path resilient-runtime/Cargo.toml --test timer_identity_regression
cargo test --manifest-path resilient-runtime/Cargo.toml --lib timer::
```

`resilient-runtime/tla/timer_identity.tla` models allocation, release, registry
reset and handle requests for two slots and two generations. TLC checks type
bounds, absence of handle resurrection, and rejection of revoked handles.
Pass `--log-dir /absolute/path/to/evidence` to retain raw TLC output and
counterexample traces. The checker also requires stale-handle counterexamples for three mutants:
missing generation validation, reset discarding generations, and generation
rollover instead of retirement.

```sh
python3 resilient-runtime/tla/tests/check_timer_identity.py --jar /absolute/path/to/tla2tools.jar
```

These are separate forms of evidence: the Rust tests exercise implementation
behavior, and TLC checks a handwritten bounded capability model. There is no
mechanized Rust-to-TLA refinement relation. The model does not describe timer
frequency arithmetic, expiry counts, deferred callback delivery, interrupt
interleavings, fairness, latency or hardware register behavior. The existing
single-core/registry-access assumptions still apply. In particular, this work
does not establish cancellation of callbacks already captured by a `tick`.

Blocking delay ownership and fair progress are covered separately in the
[timer delay model](TIMER_DELAY_MODEL.md).
