---
id: RES-101
title: Cortex-M demo crate links resilient-runtime + wires LlffHeap
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files added (all new):
- `resilient-runtime-cortex-m-demo/Cargo.toml` — binary target,
  edition 2024, release profile `opt-level = "z"`, `panic = "abort"`,
  `lto = true`. `thumbv7em-none-eabihf` deps: `cortex-m = "0.7"`
  (with `critical-section-single-core` — see deviation below),
  `cortex-m-rt = "0.7"`, `embedded-alloc = "0.5"`, and a
  path-dep on `../resilient-runtime` with `features = ["alloc"]`.
- `resilient-runtime-cortex-m-demo/.cargo/config.toml` — pins
  `target = "thumbv7em-none-eabihf"` and adds `rustflags = ["-C",
  "link-arg=-Tlink.x"]` for cortex-m-rt.
- `resilient-runtime-cortex-m-demo/memory.x` — placeholder FLASH /
  RAM map (256 KiB / 64 KiB; documented inline as
  representative-not-prescriptive).
- `resilient-runtime-cortex-m-demo/src/main.rs` — `#![no_std]` +
  `#![no_main]` + `extern crate alloc`, 4 KiB static heap
  initialised inside `#[entry] fn main() -> !`, one `Value::String`
  + one `Value::Float`, exercises `.add()` + `.eq()`, loops on
  `cortex_m::asm::nop()`. Minimal spin panic handler (no
  `panic-halt`, per the ticket's "keep deps minimal" note).
- `resilient-runtime-cortex-m-demo/README.md` — onboarding +
  flashing hints + pointer back to `../resilient-runtime`.
- `scripts/build_cortex_m_demo.sh` — idempotent `rustup target
  add` + release build + `clippy -- -D warnings`. Exits 0 on
  success. Marked executable.
- `README.md` (top-level) — new paragraph under the existing
  embedded-runtime section pointing at the demo crate.

Verification:
- `scripts/build_cortex_m_demo.sh` → exits 0.
  - `cargo build --release --target thumbv7em-none-eabihf` clean.
  - `cargo clippy --release --target thumbv7em-none-eabihf -- -D
    warnings` clean.
- `cd resilient && cargo test` — unaffected: 271 unit + 13
  integration + 1 golden still pass. The demo crate is a sibling,
  not a workspace member.

Deviations from the ticket sketch:

1. The ticket's `src/main.rs` sketch uses
   `use embedded_alloc::LlffHeap as Heap`. That rename landed in
   `embedded-alloc` 0.7; the pinned 0.5 version still exports its
   allocator as `Heap` (the `linked_list_allocator::Heap` wrapper
   under the hood). Acceptance criteria pin 0.5 explicitly, so we
   use the historical name and document the rename inline. The
   semantics (`empty()` + `init(addr, size)`) are unchanged.
2. `embedded-alloc` 0.5's `Heap::dealloc` touches a
   `critical_section::Mutex`, so the binary links only when a
   critical-section impl is provided. Enabling
   `cortex-m`'s `critical-section-single-core` feature is the
   standard wiring on a single-core M4F and adds no new
   dependency (cortex-m is already pulled in). Documented
   inline in `Cargo.toml`.

Neither deviation changes the ticket's substance — `resilient-
runtime` links cleanly under the `alloc` feature with an
`embedded-alloc` global allocator on Cortex-M4F, which is the
onboarding evidence the ticket exists to produce.
