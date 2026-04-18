//! RES-101: buildable Cortex-M4F demo that links
//! `resilient-runtime` with `embedded-alloc::LlffHeap` as the
//! `#[global_allocator]`. The goal is onboarding evidence — "yes,
//! this really does build on a Cortex-M target, here's how" — not
//! a runnable demo. Building clean is the proof.
//!
//! The binary:
//!   1. initialises a fixed-size heap (4 KiB static `[u8; N]`),
//!   2. constructs `Value::String(String::from("hello"))` and
//!      `Value::Float(2.5)`,
//!   3. exercises `.add()` and `.eq()` so the runtime ops link and
//!      don't drag in std, then
//!   4. loops forever on `cortex_m::asm::nop()`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use core::mem::MaybeUninit;

use cortex_m_rt::entry;
// `embedded-alloc` 0.5 exports its allocator as `Heap`; the ticket
// sketch mentioned `LlffHeap`, which is the 0.7+ rename — the
// acceptance criteria pin 0.5, so we use the historical name here.
// The semantics (`empty()` + `init(addr, size)`) are unchanged
// across the rename.
use embedded_alloc::Heap;
use resilient_runtime::Value;

// 4 KiB heap — plenty for the demo's single `String` and two
// `Value`s. Real firmware sizes the region against its RAM budget.
const HEAP_SIZE: usize = 4096;

#[global_allocator]
static HEAP: Heap = Heap::empty();

#[entry]
fn main() -> ! {
    // SAFETY: `HEAP` is `Heap::empty()` above; `HEAP_MEM` is an
    // uninitialised static region only this call touches, and the
    // `init` contract says it's fine to hand raw-ish uninit memory
    // over. See `embedded-alloc` 0.5 docs for the exact invariant.
    static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
    #[allow(static_mut_refs)]
    unsafe {
        HEAP.init(HEAP_MEM.as_mut_ptr() as usize, HEAP_SIZE);
    }

    // Construct one Value of each of the two runtime variants that
    // need the heap — the String proves `alloc` is wired, the Float
    // proves stack-only f64 also goes through `Value` cleanly.
    let s = Value::String(String::from("hello"));
    let f = Value::Float(2.5);

    // Exercise a couple of ops so the runtime's method impls are
    // actually linked into the final binary (otherwise LTO might
    // prune them and we'd lose the smoke-test value).
    let _ = s.clone().add(Value::String(String::from(" world")));
    let _ = f.clone().eq(Value::Float(2.5));

    loop {
        cortex_m::asm::nop();
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Minimal spin panic handler — no `panic-halt`/`defmt` so the
    // dep tree stays tight and the demo keeps its focus on the
    // runtime + allocator integration.
    loop {}
}
