//! RES-3987 (D-E1): on-device `.rzbc` loader binary.
//!
//! `docs/EMBEDDED_PIPELINE.md` section 3.3 sketches a thin no_std
//! loader template: embed a `.rzbc` blob, decode + run it, report
//! the result via semihosting. This binary is exactly that,
//! wired around the reusable `resilient_runtime::vm::loader::load_and_run`
//! (see `resilient-runtime/src/vm/loader.rs`) — it is the
//! QEMU-runnable target section 4's Cortex-M CI job (item 4 of the
//! design doc's decomposition) will exercise with
//! `qemu-system-arm -M lm3s6965evb -cpu cortex-m4
//! -semihosting-config enable=on,target=native -kernel <elf>`.
//!
//! The embedded fixture and the expected result are identical to
//! the host-side `load_and_run_committed_fixture_round_trips_through_real_decoder_and_vm`
//! test in `resilient-runtime/src/vm/loader.rs` — both consume the
//! exact same committed bytes, so a QEMU run and `cargo test` are
//! checking the same program.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use cortex_m_semihosting::{debug, hprintln};
use resilient_runtime::vm::Value;
use resilient_runtime::vm::loader::load_and_run;

/// `(2 + 3) * 4 + 1 == 21` — see
/// `resilient-runtime/fixtures/arithmetic_demo.rzbc`.
static RZBC_BLOB: &[u8] = include_bytes!("../../resilient-runtime/fixtures/arithmetic_demo.rzbc");

/// Sized for the committed fixture: 8 instructions, an operand
/// stack that never holds more than 3 values at once, and no
/// locals. A larger on-device program would need larger consts
/// here — these are compile-time `const generics` on
/// [`load_and_run`], not a runtime-configurable capacity.
const MAX_INSTRS: usize = 16;
const STACK_SLOTS: usize = 8;
const LOCALS_SLOTS: usize = 0;

const EXPECTED: Value = Value::Int(21);

#[entry]
fn main() -> ! {
    match load_and_run::<MAX_INSTRS, STACK_SLOTS, LOCALS_SLOTS>(RZBC_BLOB) {
        Ok(v) if v == EXPECTED => {
            hprintln!("loader ok: {:?}", v);
            debug::exit(debug::EXIT_SUCCESS);
        }
        Ok(v) => {
            hprintln!("loader produced unexpected value: {:?}", v);
            debug::exit(debug::EXIT_FAILURE);
        }
        Err(e) => {
            hprintln!("loader error: {:?}", e);
            debug::exit(debug::EXIT_FAILURE);
        }
    }

    loop {
        cortex_m::asm::nop();
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Minimal spin panic handler, mirroring
    // `resilient-runtime-cortex-m-demo/src/main.rs` — no
    // `panic-halt`/`defmt` dependency for a demo this small.
    loop {}
}
