---
layout: page
title: Supervisor restart budgets
nav_order: 35
permalink: /supervisor-restart-model/
---

# Supervisor restart budgets

Temporary supervisor policies enforce their restart budget both when deciding
whether to restart and when recording a restart. A zero budget disables
restarts, including after the reset interval. A rejected recording leaves the
counter and timestamp unchanged.

## Reset semantics

The current runtime measures the reset interval from the **last recorded
restart**. It resets the counter to one when that idle interval has elapsed;
it does not maintain a queue of restart timestamps or implement a sliding
window over all recent events. Clock rollback uses saturating subtraction and
does not by itself expire a positive reset interval. Positive budgets with a
zero-duration interval preserve immediate-reset behavior.

Permanent and transient policies retain their behavior: permanent counters
saturate, and transient actors are not restarted. The timed model below covers
only temporary policies; ordinary runtime tests cover those other policies.

## Runtime-to-model check

`resilient-runtime/tla/supervisor_restart.tla` models the temporary policy's
budget, interval, current count, last-restart timestamp and supplied clock.
Unlike the original actor scheduler model, it includes clock movement and
interval-dependent reset decisions. It is a separate API model, not an
extension that changes the existing scheduler configuration.

The verification driver compiles an observation program against the actual
`resilient/src/supervisor_runtime.rs` source. It checks all 288 combinations of:

- budget and reset interval from zero through two;
- an initially valid count from zero through the budget;
- current and last timestamps from zero through three, including rollback.

Each observation records the Boolean authorization result of `should_restart`,
direct `record_restart` success,
and the resulting count and timestamp. The driver rejects incomplete or
duplicate observation coverage, then supplies the observations to TLC as a
generated module. TLC checks authorization/recording correspondence, the
budget bound and disabled zero budgets across the bounded state graph.

```bash
python3 resilient-runtime/tla/tests/check_supervisor_restart.py --jar /absolute/path/to/tla2tools.jar
```

Java, a Rust 2024-capable `rustc`, and the official TLC jar are required.
No compiler build or additional Rust crate is needed. Use `--runtime-source`
to check another saved version of the implementation against the same model.
Tool failures, incomplete observations and timeouts fail the check.

The driver also requires invariant counterexamples from four temporary model
variants: allowing a zero budget, bypassing direct-recording authorization,
changing the reset boundary, and mishandling clock rollback. It never
modifies the checked-in source or model.

## Proof boundary

This establishes bounded agreement between observed Rust API behavior and a
handwritten model. It is not a proof of arbitrary integer/time domains,
compiled instructions, actor delivery, scheduler fairness or eventual service
recovery. The original scheduler model's restart counter property does not
replace this timing check, and a counter bounded within a reset interval is
not a lifetime restart cap or a convergence guarantee.

The API regression tests also cover large timestamp/counter boundaries and
state preservation on rejection. Neither those tests nor this model add a
required CI workflow or qualify the compiler under a safety standard.
