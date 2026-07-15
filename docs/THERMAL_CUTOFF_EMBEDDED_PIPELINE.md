---
title: Thermal Cutoff — Embedded Reference App
parent: Design Philosophy
nav_order: 8
permalink: /thermal-cutoff-embedded-pipeline
---

# Thermal Safety Cutoff — an End-to-End Embedded Reference App
{: .no_toc }

RES-4084 (D-E2) — the safety-critical embedded story told through one
concrete app, from source to a QEMU-run Cortex-M binary.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Why this app

Resilient exists for small, safety-critical control loops where a
single missed bound can destroy hardware or hurt someone — a
battery-management shutoff, a motor over-temperature protection, a
medical-device heater cutoff. The thermal safety cutoff is the
flagship reference app for that story (originally `PR #3850`,
`resilient/examples/thermal_safety_cutoff.rz`), and this doc extends
it through the real embedded pipeline landed in D-E1
(`docs/EMBEDDED_PIPELINE.md`): `.rz` source, compiled by `rz build`
to a `.rzbc` blob, decoded and executed by the `no_std` VM in
`resilient-runtime`, running on an actual Cortex-M target under QEMU
in CI.

The safety property under test is unchanged from the flagship app:
**the heater is never driven at or above the cutoff temperature.**
What's new here is that the property is now exercised by a real
binary on a real (emulated) microcontroller, not just the host
tree-walking interpreter.

## Two apps, two proof mechanisms

| | `thermal_safety_cutoff.rz` (flagship, PR #3850) | `thermal_cutoff_embedded.rz` (RES-4084) |
|---|---|---|
| Runs on | Host tree-walking interpreter | `rz build`'d `.rzbc` → `resilient_runtime::vm::Vm` (no_std) |
| Where the safety property is checked | Z3, at compile time, via `ensures`/`requires` contracts | Compile-time contracts on the *sibling* flagship app; this app is the runtime-executable shape of the identical control logic |
| Sensor-glitch handling | `live` block, re-reads until plausible | `safe_temp` fallback to last-known-good reading (deterministic, no retry loop) |
| I/O | `println` narrates each control step | None — the embedded `Value` has no heap-backed `String`; the program returns an `Int` |

Both encode the same decision function:

```
commanded_duty(temp, requested) =
    0                    if temp >= CUTOFF_TEMP
    clamp(requested, 0, MAX_DUTY)   otherwise
```

The flagship app is where Z3 *proves* this holds for every input
(`ensures result >= 0`, `ensures result <= 100`,
`ensures temp < 800 || result == 0`). The embedded app is where that
same logic actually *runs* on target hardware — the v1 embedded
pipeline (see below) can't yet emit `ensures`-bearing functions, so
the contract proof and the on-device execution currently live on two
sibling source files that share identical control-flow. Closing that
gap (emitting postcheck-bearing functions to the embedded VM) is
tracked in `#4083`.

## Why `thermal_cutoff_embedded.rz` looks different

The v1 embedded fn-call pipeline (`RES-4077`, `#4082`) accepts a
strict subset of what the host interpreter runs — see
`resilient/src/rzbc_emit.rs`'s module docs and `#4083` for the exact
exclusion list. The reference app is written to stay inside it:

- **Plain top-level `fn`s only** — no closures/upvalues.
- **No `fails` declarations** — the embedded `Call`/`Return` pair
  doesn't walk a try-handler table.
- **No `ensures`/`recovers_to`** on any *emitted* function — the host
  VM invokes postchecks automatically on return; the embedded
  `Instr::Return` doesn't, so a postcheck-bearing function is
  rejected at `rz build` time rather than silently dropping the
  check.
- **No strings, no `println`** — the embedded `Value` has no heap;
  every function parameter and return value is a scalar
  (`Int`/`Bool`/`Float`).
- **No `live` blocks** — `safe_temp` uses a deterministic
  last-known-good fallback instead of a retry loop, which is
  representable as a plain conditional.

The result is five small top-level functions:

```
is_plausible(temp)                         — sensor range check
commanded_duty(temp, requested)            — the cutoff decision
safe_temp(raw, last_good)                  — glitch fallback
control_step(raw_temp, last_good, requested) — one control-loop tick
main()                                     — four scripted scenarios, summed
```

`main()` exercises the same four scenarios as the flagship app
(normal operation, at cutoff, above cutoff, recovered sensor glitch)
and returns their summed duty (`100 + 0 + 0 + 80 = 180`) so a single
`Value::Int` captures pass/fail for every scenario at once.

## Pipeline stages exercised

```
resilient/examples/thermal_cutoff_embedded.rz
        │  rz build --target <TRIPLE>
        │  (resilient/src/rzbc_emit.rs: Op -> Instr, subset-checked)
        ▼
resilient-runtime/fixtures/thermal_cutoff_demo.rzbc   (.rzbc v2: main + function table)
        │  resilient_runtime::vm::serde::decode_program
        │  (no_std, fixed-capacity buffers, no heap)
        ▼
resilient_runtime::vm::Vm::run_with_functions           (no_std VM, bounded call-frame stack)
        │
        ├─ host test (resilient/tests/thermal_cutoff_embedded_pipeline.rs):
        │    cargo test decodes + runs the committed fixture directly
        │
        └─ on-device (resilient-runtime-loader-demo/src/main.rs):
             include_bytes!'d blob -> load_and_run_with_functions
             -> cortex-m-semihosting hprintln! + debug::exit()
             -> qemu-system-arm -M lm3s6965evb -cpu cortex-m4
                (resilient-runtime-loader-demo/run_qemu.sh)
```

Every stage is host-testable independently of QEMU (the embedded
`Vm` is plain `#![no_std]` Rust, no target-specific code), and the
QEMU run consumes the *identical* committed `.rzbc` bytes the host
tests decode — a green `cargo test` and a green QEMU run are checking
the same program.

## Tests

| Stage | Test |
|---|---|
| `rz build` accepts the example, for all 3 supported targets | `resilient/tests/thermal_cutoff_embedded_pipeline.rs::example_builds_for_all_supported_embedded_targets` |
| Freshly built blob decodes + runs to the expected value | `resilient/tests/thermal_cutoff_embedded_pipeline.rs::fresh_build_of_example_matches_committed_fixture` |
| Committed fixture (the exact bytes QEMU runs) decodes + runs to the expected value | `resilient/tests/thermal_cutoff_embedded_pipeline.rs::committed_fixture_matches_expected_duty_sum` |
| Embedded result matches the tree-walking interpreter oracle | `resilient/tests/thermal_cutoff_embedded_pipeline.rs::embedded_result_matches_interpreter_oracle` |
| Golden CLI output for the example (host interpreter run) | `resilient/examples/thermal_cutoff_embedded.rz` + `.expected.txt`, via `resilient/tests/it/examples_golden.rs` |
| On-device execution under QEMU | `resilient-runtime-loader-demo/run_qemu.sh` (CI: `embedded-runtime.yml`) |

## Regenerating the fixture

The committed `.rzbc` fixture is not hand-written — it's the literal
output of the host pipeline, checked in so the host tests and the
on-device binary consume identical bytes:

```sh
cargo run --manifest-path resilient/Cargo.toml --bin rz -- \
  build --target thumbv7em-none-eabihf \
  resilient/examples/thermal_cutoff_embedded.rz \
  -o resilient-runtime/fixtures/thermal_cutoff_demo.rzbc
```

## See also

- `docs/EMBEDDED_PIPELINE.md` — the D-E1 design doc for the pipeline
  this app exercises.
- `docs/REFERENCE_APP_THERMAL_CUTOFF.md` — the host-only flagship
  app's contracts-and-`live`-blocks story.
- `resilient-runtime-loader-demo/README.md` — the loader binary and
  QEMU harness this app is wired into.
- `#4083` — tracked v1 exclusions (closures, `fails`, postchecks,
  packed locals) this app deliberately avoids; this doc's QEMU
  fn-fixture item is now closed by RES-4084, but the ticket stays
  open for the rest of its scope.
