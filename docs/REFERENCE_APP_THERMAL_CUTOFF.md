---
title: "Reference App: Thermal Safety Cutoff"
parent: Language Reference
nav_order: 32
permalink: /reference-app/thermal-cutoff
---

# Reference Application: Thermal Safety Cutoff
{: .no_toc }

The flagship example of what Resilient is *for* — and who should reach
for it.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Who is this for?

If you write firmware where a single missed bound can destroy hardware
or hurt someone, this page is the pitch. Concretely:

- **Battery-management engineers** who must guarantee a heater or
  charger is cut off before a cell crosses a thermal limit.
- **Motor / power-electronics firmware** teams enforcing an
  over-temperature or over-current shutoff.
- **Medical-device developers** (IEC 62304) building a heater, pump, or
  actuator cutoff that has to be argued to an auditor.

You do **not** need Resilient to build a whole product in it. The value
shows up when you carve out the *one small component* that must not
fail — the safety interlock — and write **that** in Resilient, so the
proof ships with the binary. Everything else can stay in C, Rust, or
Ada.

## The component

[`thermal_safety_cutoff.rz`](https://github.com/EricSpencer00/Resilient/blob/main/resilient/examples/thermal_safety_cutoff.rz)
is a self-contained thermal cutoff controller. Temperatures are in
tenths of a degree C so the whole loop stays in integer arithmetic — no
FPU required on the smallest targets.

It leans on exactly two Resilient guarantees.

### 1. Contracts encode the safety property

```rust
fn commanded_duty(int temp, int requested) -> int
    requires requested >= 0
    requires requested <= 100
    ensures result >= 0
    ensures result <= 100
    ensures temp < 800 || result == 0
{
    if temp >= 800 {
        return 0;
    }
    return requested;
}
```

The three `ensures` clauses **are** the safety certificate:

1. the commanded duty is always a valid PWM value (`0 ..= 100`), and
2. at or above the cutoff temperature (`800` = 80.0 °C) the duty is
   provably zero — `temp < 800 || result == 0` is the disjunctive form
   of "temp ≥ cutoff **implies** duty = 0".

With `--features z3` these are discharged by the SMT solver for *every*
input, then recorded in a
[certificate manifest]({{ '/certificates' | relative_url }}) you can
hand downstream. A reviewer re-runs `rz verify-all` and re-checks the
proof — they do not have to trust your code review.

> **Why literals, not the `static let` names, inside the contract?**
> Inlining `100` and `800` keeps the SMT translation fully concrete, so
> the solver reasons about numbers rather than opaque global symbols.
> The named constants still drive the runtime logic; the contract just
> restates their values so the proof stands alone.

### 2. Live blocks absorb sensor glitches

Real sensors throw transient garbage — open circuits, EMI, ADC hiccups.
A bad read must never crash the loop *or* be trusted as a real
temperature. The read happens inside a `live` block guarded by a
plausibility check:

```rust
fn safe_read(int nominal) -> int {
    live invariant true {
        let reading = read_temp_sensor(nominal);
        if !is_plausible(reading) {
            assert(false, "implausible sensor reading — retrying");
        }
        return reading;
    }
}
```

When the plausibility assertion fails, the live block rolls back to the
last known-good state and re-samples. Control only proceeds once the
reading is inside the sensor's rated band, so a glitch costs a retry,
not a wrong decision.

## Running it

```bash
# Plain run — exercises the control loop and self-healing.
cargo run --manifest-path resilient/Cargo.toml -- \
    resilient/examples/thermal_safety_cutoff.rz

# With the SMT verifier — discharges the cutoff contracts.
cargo run --manifest-path resilient/Cargo.toml --features z3 -- \
    resilient/examples/thermal_safety_cutoff.rz
```

Expected output (also the golden sidecar
[`thermal_safety_cutoff.expected.txt`](https://github.com/EricSpencer00/Resilient/blob/main/resilient/examples/thermal_safety_cutoff.expected.txt),
checked in CI):

```text
=== Thermal Safety Cutoff Controller ===
temp=720 requested=100 -> duty=100
temp=800 requested=100 -> duty=0
temp=910 requested=60 -> duty=0
temp=500 requested=80 -> duty=80
recovered sensor glitches: 8
cutoff safety property encoded as machine-checked contracts
```

Read the three interesting lines:

- `temp=800 … duty=0` — exactly at the cutoff, the heater is off.
- `temp=910 … duty=0` — above the cutoff, still off, even though 60 was
  requested. The contract makes any other outcome un-compilable.
- `recovered sensor glitches: 8` — four control steps, two glitchy reads
  each, every one absorbed by a live-block retry.

## Where this goes next

This example runs on the host interpreter. The same runtime primitives
cross-compile to Cortex-M and RISC-V bare metal — see the
[`resilient-runtime-cortex-m-demo`](https://github.com/EricSpencer00/Resilient/tree/main/resilient-runtime-cortex-m-demo)
for the `#![no_std]` link proof. The honest gap today is a full
board-level build of *this* controller driving real PWM; that is the
next milestone, and it is deliberately small precisely because the
safety-critical surface is small.

See also: [Failure Model]({{ '/failure-model' | relative_url }}) ·
[Certificate Manifest Schema]({{ '/certificates' | relative_url }}) ·
[IEC 62304 mapping](https://ericspencer.us/Resilient/standards/iec-62304)
