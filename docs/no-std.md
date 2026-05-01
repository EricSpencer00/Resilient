---
title: no_std Runtime
parent: Design Philosophy
nav_order: 2
permalink: /no-std
---

# `#![no_std]` Runtime
{: .no_toc }

Embedding Resilient on a Cortex-M class MCU.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## What it is

Resilient ships a sibling crate at
[`resilient-runtime/`](https://github.com/EricSpencer00/Resilient/tree/main/resilient-runtime)
that carves out the value layer + core ops in a
`#![no_std]`-compatible form. It's verified to cross-compile to
`thumbv7em-none-eabihf` (Cortex-M4F class MCU) in both feature
configs.

This is the foundation for running Resilient programs on a
microcontroller. The host build (`resilient/`) uses the full
interpreter / VM / JIT; the embedded build uses just this
runtime crate plus a future `Program` evaluator.

## Feature configs

| Feature           | Adds                                | Use when                              |
|-------------------|-------------------------------------|---------------------------------------|
| (default)         | `Value::Int`, `Value::Bool`, `Value::Float` | Stack-only types, no allocator needed |
| `--features alloc`| `Value::String`                     | When you need string values; pulls in `embedded-alloc` |

The `alloc` feature does NOT pick a `#[global_allocator]` — that's
the binary's responsibility (see below).

## Build for host

```bash
cd resilient-runtime

# Default (alloc-free) — 11 unit tests
cargo build
cargo test

# With alloc — 14 unit tests (adds Float + String coverage)
cargo build --features alloc
cargo test  --features alloc
```

## Cross-compile to Cortex-M4F

```bash
rustup target add thumbv7em-none-eabihf

# Default — Cortex-M4F has native i64 instruction support, no
# compiler_builtins shim needed.
cargo build  --target thumbv7em-none-eabihf
cargo clippy --target thumbv7em-none-eabihf -- -D warnings

# With --features alloc, embedded-alloc 0.5 is pulled in.
cargo build  --target thumbv7em-none-eabihf --features alloc
cargo clippy --target thumbv7em-none-eabihf --features alloc -- -D warnings
```

## Wiring an allocator (binary side)

```rust
#![no_std]
#![no_main]

extern crate alloc;

use embedded_alloc::LlffHeap as Heap;
#[global_allocator]
static HEAP: Heap = Heap::empty();

#[cortex_m_rt::entry]
fn main() -> ! {
    use core::mem::MaybeUninit;
    const HEAP_SIZE: usize = 4096;
    static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] =
        [MaybeUninit::uninit(); HEAP_SIZE];
    unsafe { HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE); }

    // Now resilient_runtime::Value::String / Float work.
    use resilient_runtime::Value;
    let _ = Value::Float(2.5).add(Value::Float(1.5));

    loop { cortex_m::asm::nop(); }
}
```

A buildable example crate is planned (RES-101). Until then the
sketch above is the canonical pattern.

## Value semantics

The runtime mirrors the host VM's semantics so a program runs
identically on either backend:

```rust
use resilient_runtime::Value;

let r = Value::Int(2).add(Value::Int(3))?;       // → Value::Int(5)
let r = Value::Int(i64::MAX).add(Value::Int(1))?; // wrapping → Value::Int(i64::MIN)
let e = Value::Int(10).div(Value::Int(0));        // → Err(RuntimeError::DivideByZero)
let e = Value::Int(1).add(Value::Bool(true));     // → Err(RuntimeError::TypeMismatch("add"))
```

- Int arithmetic wraps on overflow (matches the bytecode VM).
- Float follows IEEE-754 (`1.0 / 0.0 == inf`, NaN equals itself
  for bit-equality consistency with the constant pool).
- Mixed-type ops are a `TypeMismatch` — promotion is the
  caller's job.

## Roadmap

The runtime is the foundation for the long-term plan of running
Resilient programs on bare-metal MCUs. Concretely:

1. **RES-075/097/098** ✅ — value layer + cross-compile +
   `alloc` feature
2. **RES-101** (open) — buildable Cortex-M demo crate with
   `LlffHeap` and a `#[entry]` function
3. **Future** — port a subset of the bytecode VM into
   `resilient-runtime` so embedded programs can run pre-compiled
   bytecode without a host toolchain
4. **Future** — `live { }` block semantics with explicit
   snapshot/restore for embedded I/O effects

See [ROADMAP.md](https://github.com/EricSpencer00/Resilient/blob/main/ROADMAP.md)
goalpost G18 for status.

## Hello, GPIO — Volatile MMIO and Interrupt Handlers

Volatile MMIO lets you write Resilient code that reads from and writes to memory-mapped hardware registers on a microcontroller. The compiler enforces that all volatile access is wrapped in `unsafe` blocks, and the `#[interrupt]` attribute lets you define interrupt service routines that the runtime's vector table links automatically.

```resilient
const GPIOA_ODR: Int = 0x4001_0C14;  # GPIO output data register
const SYSTICK_CSR: Int = 0xE000_E010; # SysTick control register

unsafe fn write_led_on() {
    volatile_write_u32(GPIOA_ODR, 1);
}

unsafe fn write_led_off() {
    volatile_write_u32(GPIOA_ODR, 0);
}

#[interrupt(name = "SysTick")]
fn tick_handler() {
    unsafe { write_led_off(); }
}

fn main() {
    write_led_on();
}
```

Build with:
```bash
cargo build --release --target thumbv7em-none-eabihf --manifest-path resilient-runtime-cortex-m-demo/Cargo.toml
```

The compiler lowers `#[interrupt(name = "SysTick")]` to an external symbol `__resilient_isr_SysTick` marked `extern "C"` and `no_mangle`. The `resilient-runtime-cortex-m-demo` crate provides a vector table with weak aliases that resolve to this symbol, so your interrupt handler is automatically registered without manual symbol manipulation.
