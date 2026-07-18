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
//!
//! RES-4084 (D-E2): a second blob is embedded and run below — the
//! thermal-safety-cutoff reference app
//! (`resilient/examples/thermal_cutoff_embedded.rz`), compiled by
//! `rz build` into the v2 function-table `.rzbc` format and run via
//! [`resilient_runtime::vm::loader::load_and_run_with_functions`].
//! This is the fn-calling QEMU smoke test tracked as a pending item
//! in #4083 — a function-decomposed control loop (sensor
//! plausibility check, cutoff decision, control step, each a
//! top-level `fn`) rather than the flat arithmetic fixture above.
//! See `docs/THERMAL_CUTOFF_EMBEDDED_PIPELINE.md` for the full
//! source -> contracts-checked -> bytecode -> no_std VM story.
//!
//! RES-4083 (D-E1 tail): a third blob is embedded and run below —
//! `resilient/examples/fails_try_embedded.rz`, a `fails`/
//! `try { } catch { }` checked-failure program, run via
//! [`resilient_runtime::vm::loader::load_and_run_with_functions_and_tries`].
//! This is the QEMU-runnable proof that checked-failure dispatch
//! (the deterministic catch-arm injection in
//! `resilient_runtime::vm::Vm::execute`'s `Instr::Call` arm) works on
//! real hardware, not just under `cargo test`.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use cortex_m_semihosting::{debug, hprintln};
use resilient_runtime::vm::Value;
use resilient_runtime::vm::loader::{
    load_and_run, load_and_run_with_functions, load_and_run_with_functions_and_tries,
};

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

/// The thermal-cutoff reference app — see
/// `resilient/examples/thermal_cutoff_embedded.rz` and
/// `resilient-runtime/fixtures/thermal_cutoff_demo.rzbc`. Four
/// `control_step` calls summed: 100 (normal) + 0 (at cutoff) + 0
/// (above cutoff) + 80 (glitch recovered via last-known-good) = 180.
static THERMAL_RZBC_BLOB: &[u8] =
    include_bytes!("../../resilient-runtime/fixtures/thermal_cutoff_demo.rzbc");

/// Sized generously against the committed fixture (5 top-level fns,
/// shallow non-recursive call depth, a handful of locals per frame).
const THERMAL_MAIN_N: usize = 32;
const THERMAL_FUNC_META_N: usize = 8;
const THERMAL_FUNC_CODE_N: usize = 96;
const THERMAL_STACK: usize = 16;
const THERMAL_LOCALS: usize = 8;
const THERMAL_CALLS: usize = 6;

const THERMAL_EXPECTED: Value = Value::Int(180);

/// The checked-failure / try-catch reference program — see
/// `resilient/examples/fails_try_embedded.rz`. `read_sensor` declares
/// `fails Timeout`; the call inside `try { }` always dispatches to
/// `catch Timeout` (the embedded VM injects the checked failure
/// deterministically), so the result is always `-1`.
static FAILS_TRY_RZBC_BLOB: &[u8] =
    include_bytes!("../../resilient-runtime/fixtures/fails_try_demo.rzbc");

const FAILS_TRY_MAIN_N: usize = 16;
const FAILS_TRY_FUNC_META_N: usize = 4;
const FAILS_TRY_FUNC_CODE_N: usize = 32;
const FAILS_TRY_TRY_META_N: usize = 1;
const FAILS_TRY_STACK: usize = 8;
const FAILS_TRY_LOCALS: usize = 4;
const FAILS_TRY_CALLS: usize = 3;
const FAILS_TRY_TRIES: usize = 1;

const FAILS_TRY_EXPECTED: Value = Value::Int(-1);

#[entry]
fn main() -> ! {
    match load_and_run::<MAX_INSTRS, STACK_SLOTS, LOCALS_SLOTS>(RZBC_BLOB) {
        Ok(v) if v == EXPECTED => {
            hprintln!("loader ok: {:?}", v);
        }
        Ok(v) => {
            hprintln!("loader produced unexpected value: {:?}", v);
            debug::exit(debug::EXIT_FAILURE);
            loop {
                cortex_m::asm::nop();
            }
        }
        Err(e) => {
            hprintln!("loader error: {:?}", e);
            debug::exit(debug::EXIT_FAILURE);
            loop {
                cortex_m::asm::nop();
            }
        }
    }

    match load_and_run_with_functions::<
        THERMAL_MAIN_N,
        THERMAL_FUNC_META_N,
        THERMAL_FUNC_CODE_N,
        THERMAL_STACK,
        THERMAL_LOCALS,
        THERMAL_CALLS,
    >(THERMAL_RZBC_BLOB)
    {
        Ok(v) if v == THERMAL_EXPECTED => {
            hprintln!("thermal cutoff loader ok: {:?}", v);
        }
        Ok(v) => {
            hprintln!("thermal cutoff loader produced unexpected value: {:?}", v);
            debug::exit(debug::EXIT_FAILURE);
            loop {
                cortex_m::asm::nop();
            }
        }
        Err(e) => {
            hprintln!("thermal cutoff loader error: {:?}", e);
            debug::exit(debug::EXIT_FAILURE);
            loop {
                cortex_m::asm::nop();
            }
        }
    }

    match load_and_run_with_functions_and_tries::<
        FAILS_TRY_MAIN_N,
        FAILS_TRY_FUNC_META_N,
        FAILS_TRY_FUNC_CODE_N,
        FAILS_TRY_TRY_META_N,
        FAILS_TRY_STACK,
        FAILS_TRY_LOCALS,
        FAILS_TRY_CALLS,
        FAILS_TRY_TRIES,
    >(FAILS_TRY_RZBC_BLOB)
    {
        Ok(v) if v == FAILS_TRY_EXPECTED => {
            hprintln!("fails/try loader ok: {:?}", v);
            debug::exit(debug::EXIT_SUCCESS);
        }
        Ok(v) => {
            hprintln!("fails/try loader produced unexpected value: {:?}", v);
            debug::exit(debug::EXIT_FAILURE);
        }
        Err(e) => {
            hprintln!("fails/try loader error: {:?}", e);
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
