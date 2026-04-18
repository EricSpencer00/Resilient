# resilient-runtime-cortex-m-demo (RES-101)

Buildable Cortex-M4F demo that links `resilient-runtime` with
`embedded-alloc::Heap` as the `#[global_allocator]`. The goal is
**onboarding evidence** — "yes, this really does build on a
Cortex-M target, here's how" — not a runnable demo. Building
clean is the proof. We deliberately do **not** run the output
under QEMU in CI.

## Target class

- **CPU**: ARM Cortex-M4F (Thumb-2 + FPv4-SP hardware float).
- **Rust target triple**: `thumbv7em-none-eabihf`.
- **Memory map placeholder**: 256 KiB FLASH at `0x08000000`,
  64 KiB RAM at `0x20000000`. Override `memory.x` if your board
  differs — the values are only enough for the linker to succeed
  in this demo.

Representative boards that match this class: STM32F4xx, STM32F7xx
(Cortex-M4 portion), nRF52-family with DCWP matching M4F.

## Building

From the repo root:

```sh
scripts/build_cortex_m_demo.sh
```

The script installs the `thumbv7em-none-eabihf` toolchain
(idempotent no-op if already present), then runs
`cargo build --release --target thumbv7em-none-eabihf` and a
`cargo clippy -- -D warnings` pass. Exits 0 on a clean build.

## Flashing (optional)

This crate is a build check, not a runtime demo — but if you have a
Cortex-M4F dev board and want to try it:

```sh
# probe-rs flash (install via `cargo install probe-rs --features cli`)
cd resilient-runtime-cortex-m-demo
probe-rs run --chip <your-chip> \
  target/thumbv7em-none-eabihf/release/resilient-runtime-cortex-m-demo
```

The binary allocates a 4 KiB heap, constructs one `Value::String`
and one `Value::Float`, exercises `.add()` and `.eq()`, then spins
forever on `cortex_m::asm::nop()`. There is no output — the
exercise is purely to prove the runtime ops link on-target without
dragging in std.

## See also

`../resilient-runtime/README.md` (upstream value-layer docs) and
`../resilient/README.md`'s embedded-runtime section for the
broader context of why this crate exists.
