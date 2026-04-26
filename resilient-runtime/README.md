# resilient-runtime

`#![no_std]` runtime for the Resilient language. Carries the
`Value` enum, core arithmetic / equality ops, the `Sink`
abstraction for telemetry, and an optional FFI static registry.

## Features

| Feature | Effect |
|---|---|
| (default) | Allocation-free. `Value` is `Int` / `Bool` / `Float`. |
| `alloc` | Adds `Value::String`. Pulls in `embedded-alloc`; user wires `#[global_allocator]`. |
| `static-only` | Asserts no-heap posture. Mutually exclusive with `alloc`. |
| `std-sink` | Adds `StdoutSink` for host-side telemetry routing. |
| `ffi-static` | FFI static registry (pick exactly one capacity flag). |

## Heap profiler (RES-374)

Under `--features alloc`, `resilient_runtime::heap` exposes a thin
shim around the user's `#[global_allocator]` so firmware can
observe peak heap usage and stay within budget.

### API

```rust
resilient_runtime::heap::peak_bytes() -> usize     // high-water mark since boot / last reset
resilient_runtime::heap::current_bytes() -> usize  // bytes currently held by live allocations
resilient_runtime::heap::reset_peak()              // reset peak to current
```

All three are available without the `alloc` feature too — they
return `0` / no-op so portable code compiles in both postures.

### Wiring on a Cortex-M / RISC-V target

```rust
use resilient_runtime::heap::TrackingHeap;
use embedded_alloc::Heap;

#[global_allocator]
static HEAP: TrackingHeap<Heap> = TrackingHeap::new(Heap::empty());

#[entry]
fn main() -> ! {
    // Initialise the inner heap once at boot.
    static mut HEAP_MEM: [MaybeUninit<u8>; 4096] = [MaybeUninit::uninit(); 4096];
    unsafe { HEAP.inner().init(HEAP_MEM.as_mut_ptr() as usize, 4096); }

    // ... do work that allocates ...

    let peak = resilient_runtime::heap::peak_bytes();
    // route `peak` through your telemetry sink.

    loop {}
}
```

`TrackingHeap` is `const`-constructible and forwards every
`GlobalAlloc` method to the inner allocator while updating two
`AtomicUsize` counters with `Ordering::Relaxed`. It adds two
atomic operations per allocation and one CAS-loop bubble for the
peak — negligible for embedded workloads.
