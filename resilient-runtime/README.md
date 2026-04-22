# resilient-runtime

Minimal `#![no_std]` runtime types for Resilient — suitable for embedded
targets. Ships the `Value` enum + core arithmetic/equality ops.

## Features

| Feature        | Adds                                          | Use when                                                   |
|----------------|-----------------------------------------------|------------------------------------------------------------|
| *(default)*    | `Value::Int`, `Value::Bool`, `Value::Float`   | Stack-only types; no allocator needed                      |
| `alloc`        | `Value::String`, heap profiler                | When you need string values; pulls in `embedded-alloc`     |
| `static-only`  | *(nothing added)*                             | Assertive ban on heap use; compile-errors if `alloc` is also enabled |
| `std-sink`     | `StdoutSink`                                  | Convenience sink for `std` environments                    |
| `ffi-static`   | FFI static registry                           | Host-side FFI without dynamic allocation                   |

## Quick start

```bash
# Default (alloc-free) build
cargo build
cargo test

# With alloc (enables Value::String and heap profiler)
cargo build --features alloc
cargo test  --features alloc
```

## Heap profiler (`--features alloc`) — RES-374

When the `alloc` feature is enabled you can observe peak heap usage via
the `resilient_runtime::heap` module. This is essential for embedded
systems that must stay within a fixed heap budget.

### API

```rust
use resilient_runtime::heap;

// Read the peak heap allocation since program start (or last reset).
let peak: usize = heap::peak_bytes();

// Reset the high-water mark to the current live allocation size.
heap::reset_peak();
```

`peak_bytes()` returns `0` on default (no-alloc) builds, so call sites
need no feature-gating.

### Wiring: `ProfilingAllocator`

Wrap your `#[global_allocator]` with `ProfilingAllocator` to enable
automatic tracking:

```rust
#![no_std]
#![no_main]

extern crate alloc;

use embedded_alloc::Heap;
use resilient_runtime::heap::ProfilingAllocator;

#[global_allocator]
static HEAP: ProfilingAllocator<Heap> = ProfilingAllocator::new(Heap::empty());

#[cortex_m_rt::entry]
fn main() -> ! {
    use core::mem::MaybeUninit;
    const HEAP_SIZE: usize = 4096;
    static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] =
        [MaybeUninit::uninit(); HEAP_SIZE];
    // SAFETY: single-threaded init; HEAP_MEM is only touched here.
    unsafe { HEAP.0.init(HEAP_MEM.as_mut_ptr() as usize, HEAP_SIZE); }

    // ... do work ...

    let peak = resilient_runtime::heap::peak_bytes();
    // assert!(peak <= MY_HEAP_BUDGET);

    loop {}
}
```

`ProfilingAllocator` is a zero-overhead transparent wrapper — it forwards
every `alloc`/`dealloc` call to the inner allocator and updates two
`AtomicUsize` counters (`CURRENT_BYTES` and `PEAK_BYTES`). There is no
locking; `Relaxed` atomic ordering is sufficient for monotone
high-water-mark tracking.

### How it works

| Counter        | Updated by      | Read by        |
|----------------|-----------------|----------------|
| `CURRENT_BYTES`| alloc / dealloc | `reset_peak`   |
| `PEAK_BYTES`   | `record_alloc`  | `peak_bytes`   |

On every successful allocation `record_alloc` adds the layout size to
`CURRENT_BYTES` and then performs a CAS loop to raise `PEAK_BYTES` if
`CURRENT_BYTES` now exceeds the stored peak. On deallocation
`record_dealloc` subtracts the size from `CURRENT_BYTES`.

### Embedded cross-compile

```bash
cd resilient-runtime
cargo build \
  --target thumbv7em-none-eabihf \
  --features alloc
```

The heap profiler is `no_std` compatible — `AtomicUsize` is available on
all Cortex-M and RISC-V targets supported by the runtime.
