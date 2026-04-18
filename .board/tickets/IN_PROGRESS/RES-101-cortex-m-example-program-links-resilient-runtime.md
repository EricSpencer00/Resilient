---
id: RES-101
title: Cortex-M demo crate links resilient-runtime + wires LlffHeap
state: OPEN
priority: P3
goalpost: G18
created: 2026-04-17
owner: executor
---

## Summary
RES-097 proved `resilient-runtime` cross-compiles to
`thumbv7em-none-eabihf` (Cortex-M4F). RES-098 added the opt-in
`alloc` feature so `Value::String` works on-target with a
heap. The README documents the pattern users must follow to wire
a `#[global_allocator]`. This ticket makes that pattern
buildable: a tiny example crate that links `resilient-runtime`
with `--features alloc`, picks `embedded-alloc::LlffHeap` as
the global allocator, and exercises a few `Value` ops in
`#[entry]`.

The goal is onboarding evidence — "yes, this really does build
on a Cortex-M target, here's how" — not a runnable demo. We
won't run it under QEMU in CI. Building clean is the proof.

## Acceptance criteria
- New crate at `resilient-runtime-cortex-m-demo/` (sibling of
  `resilient-runtime/`):
  - `Cargo.toml` declares a binary target, edition 2024.
  - `[target.thumbv7em-none-eabihf]` deps: `cortex-m`,
    `cortex-m-rt`, `embedded-alloc = "0.5"`, plus a path
    dependency on `../resilient-runtime` with `features =
    ["alloc"]`.
  - `[profile.release]` opt-level = "z" (size, since this is a
    demo for embedded), `panic = "abort"`, `lto = true`. Don't
    fuss with `[profile.dev]` — release is what proves the
    integration.
  - `.cargo/config.toml` pins `target = "thumbv7em-none-eabihf"`
    and adds `rustflags = ["-C", "link-arg=-Tlink.x"]` so
    cortex-m-rt's linker script gets picked up.
  - `memory.x` at the crate root with placeholder FLASH/RAM
    sections matching a generic Cortex-M4 (e.g. 256K FLASH,
    64K RAM). Concrete values are board-specific; ours are
    just enough for the linker to succeed.
- `src/main.rs`:
  - `#![no_std]` + `#![no_main]` + `extern crate alloc;`
  - `use embedded_alloc::LlffHeap as Heap;` and
    `#[global_allocator] static HEAP: Heap = Heap::empty();`
  - `#[cortex_m_rt::entry] fn main() -> !` that:
    1. Initializes HEAP with a fixed-size memory pool from a
       static `[u8; N]` (N = 4096 is plenty for the demo).
    2. Constructs `Value::String(String::from("hello"))` and
       `Value::Float(2.5)`.
    3. Calls `.add()` and `.eq()` on those values to prove the
       runtime ops link and don't drag in std.
    4. Drops results (`let _ = ...`) and enters
       `loop { cortex_m::asm::nop(); }`.
  - `#[panic_handler]` that just spins (`loop {}`) — embedded
    binaries need one and we don't ship `panic-halt` to keep
    deps minimal.
- A short shell script at repo root `scripts/build_cortex_m_demo.sh`
  that runs:
  ```sh
  rustup target add thumbv7em-none-eabihf >/dev/null
  cd resilient-runtime-cortex-m-demo
  cargo build --release --target thumbv7em-none-eabihf
  cargo clippy --release --target thumbv7em-none-eabihf -- -D warnings
  ```
  Exits 0 on success. Documents what RES-101 is checking.
- README.md at the demo crate root explains: what board class
  this targets, how to flash if the user has a Cortex-M4F
  board, and points back to `resilient-runtime/README.md` for
  the underlying value-layer docs.
- Top-level `README.md`: add a paragraph under the existing
  embedded section pointing at the new demo crate ("see
  `resilient-runtime-cortex-m-demo/` for a buildable example
  that links the runtime with LlffHeap").
- All four feature configs of `resilient/` itself are
  unchanged — this ticket adds a new sibling crate only.
- Manual verification step: run the script. Builds clean, clippy
  clean. (We don't add this to CI yet — it requires `rustup
  target add` which mutates the host toolchain.)
- Commit message: `RES-101: cortex-m demo crate links resilient-runtime + LlffHeap (G18)`.

## Notes
- The demo isn't a workspace member of `resilient/`'s
  Cargo.toml — it's a separate Cargo project, same as
  `resilient-runtime/` itself. A future ticket can promote
  all three to a workspace if there's a real reason
  (shared profile, cross-crate testing).
- `LlffHeap::empty()` followed by `HEAP.init(...)` in main is
  the standard pattern; `embedded-alloc` 0.5's docs cover the
  exact signatures. Don't drift from those.
- `memory.x` placeholder values: the linker only cares that
  origin/length make sense. Real values come from the user's
  chip datasheet — we explicitly note this in the demo README.
- Don't add `defmt` / `cortex-m-semihosting` — those would
  pull in extra deps and make the demo less obviously about
  resilient-runtime. Keep it minimal.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager
