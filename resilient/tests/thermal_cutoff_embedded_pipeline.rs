//! RES-4084 (D-E2): end-to-end proof for the thermal-cutoff
//! reference app through the real embedded pipeline —
//! `resilient/examples/thermal_cutoff_embedded.rz` compiled by
//! `rz build` to a `.rzbc` v2 (function-table) blob, decoded with
//! `resilient_runtime::vm::serde::decode_program`, and executed on
//! the no_std `resilient_runtime::vm::Vm`. This is a standalone
//! `tests/*.rs` binary (not wired into `tests/it/main.rs`) so it
//! doesn't need to touch that file.
//!
//! Three things are checked:
//!   1. `rz build --target <TRIPLE>` accepts the example (stays
//!      inside the v1 embedded fn-call subset — no closures, no
//!      `fails`, no `ensures`/`recovers_to`).
//!   2. The freshly-built blob decodes and runs on the embedded VM
//!      to the same value as the tree-walking interpreter running
//!      the identical control logic (the differential oracle).
//!   3. The *committed* fixture
//!      (`resilient-runtime/fixtures/thermal_cutoff_demo.rzbc`,
//!      the exact bytes `resilient-runtime-loader-demo` embeds via
//!      `include_bytes!` and QEMU-runs) round-trips to the same
//!      result — so a `cargo test` run and a QEMU run are checking
//!      the same program, mirroring the pattern
//!      `resilient-runtime/src/vm/loader.rs`'s doc comments
//!      describe for the arithmetic fixture.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use resilient_runtime::vm::serde::{DecodedFunctionMeta, decode_program};
use resilient_runtime::vm::{FunctionDef, Instr, Value, Vm};

const EXPECTED_DUTY_SUM: Value = Value::Int(180); // 100 + 0 + 0 + 80

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_thermal_embedded_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

fn run_and_decode(blob: &[u8]) -> Value {
    let mut out_main = [Instr::Return; 64];
    let mut out_func_meta = [DecodedFunctionMeta {
        offset: 0,
        len: 0,
        arity: 0,
        local_count: 0,
        postcheck: None,
    }; 8];
    let mut out_func_code = [Instr::Return; 128];

    let counts = decode_program(blob, &mut out_main, &mut out_func_meta, &mut out_func_code)
        .expect("thermal cutoff blob should decode as the v2 function-table format");

    let mut functions = [FunctionDef {
        code: &[][..],
        arity: 0,
        local_count: 0,
        postcheck: None,
    }; 8];
    for (slot, meta) in functions
        .iter_mut()
        .zip(out_func_meta.iter())
        .take(counts.func_count)
    {
        *slot = FunctionDef {
            code: &out_func_code[meta.offset as usize..(meta.offset + meta.len) as usize],
            arity: meta.arity,
            local_count: meta.local_count,
            postcheck: meta.postcheck,
        };
    }

    let mut vm = Vm::<32, 16, 8>::new();
    vm.run_with_functions(
        &functions[..counts.func_count],
        &out_main[..counts.main_len],
    )
    .expect("embedded VM should run the thermal cutoff control loop without error")
}

/// The committed fixture — identical bytes to what
/// `resilient-runtime-loader-demo` embeds and QEMU runs.
#[test]
fn committed_fixture_matches_expected_duty_sum() {
    let blob = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../resilient-runtime/fixtures/thermal_cutoff_demo.rzbc"
    ))
    .expect("committed thermal_cutoff_demo.rzbc fixture should be readable");
    assert!(blob.starts_with(b"RZBC"));
    assert_eq!(run_and_decode(&blob), EXPECTED_DUTY_SUM);
}

/// Freshly built from source, proving the pipeline still closes
/// (not just that the committed bytes happen to work).
#[test]
fn fresh_build_of_example_matches_committed_fixture() {
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("thermal_cutoff_embedded.rz");
    let dir = tmp_dir("fresh_build");
    let out = dir.join("thermal_cutoff_embedded.rzbc");

    let output = Command::new(bin())
        .args(["build", "--target", "thumbv7em-none-eabihf"])
        .arg(&example)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("spawn rz build");
    assert_eq!(
        output.status.code(),
        Some(0),
        "rz build should accept the thermal cutoff example; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let blob = std::fs::read(&out).expect("rz build should have written the .rzbc file");
    assert_eq!(run_and_decode(&blob), EXPECTED_DUTY_SUM);

    let _ = std::fs::remove_dir_all(&dir);
}

/// The example must also build clean for the other two supported
/// embedded targets — the control logic has no target-specific
/// construct, so this is mostly a regression guard against a
/// target-triple-conditional bug creeping into `rzbc_emit.rs`.
#[test]
fn example_builds_for_all_supported_embedded_targets() {
    for target in ["thumbv6m-none-eabi", "riscv32imac-unknown-none-elf"] {
        let example = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join("thermal_cutoff_embedded.rz");
        let dir = tmp_dir(target);
        let out = dir.join("out.rzbc");

        let output = Command::new(bin())
            .args(["build", "--target", target])
            .arg(&example)
            .arg("-o")
            .arg(&out)
            .output()
            .expect("spawn rz build");
        assert_eq!(
            output.status.code(),
            Some(0),
            "rz build should accept the thermal cutoff example for target {target}; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Cross-checks the embedded VM's result against the tree-walking
/// interpreter running the same control-flow logic with `println`
/// added (the embedded subset has no I/O, so the interpreter
/// reference needs a print-wrapped sibling — mirrors the pattern in
/// `resilient/tests/it/rzbc_build_roundtrip.rs`).
#[test]
fn embedded_result_matches_interpreter_oracle() {
    let dir = tmp_dir("oracle");
    let src = dir.join("thermal_oracle.rz");
    std::fs::write(
        &src,
        "fn is_plausible(int temp) -> bool {\n\
         \x20   if temp < -400 { return false; }\n\
         \x20   if temp > 1250 { return false; }\n\
         \x20   return true;\n\
         }\n\
         fn commanded_duty(int temp, int requested) -> int {\n\
         \x20   if temp >= 800 { return 0; }\n\
         \x20   if requested > 100 { return 100; }\n\
         \x20   if requested < 0 { return 0; }\n\
         \x20   return requested;\n\
         }\n\
         fn safe_temp(int raw, int last_good) -> int {\n\
         \x20   if is_plausible(raw) { return raw; }\n\
         \x20   return last_good;\n\
         }\n\
         fn control_step(int raw_temp, int last_good, int requested) -> int {\n\
         \x20   let temp = safe_temp(raw_temp, last_good);\n\
         \x20   return commanded_duty(temp, requested);\n\
         }\n\
         fn run() -> int {\n\
         \x20   let step1 = control_step(720, 720, 100);\n\
         \x20   let step2 = control_step(800, 720, 100);\n\
         \x20   let step3 = control_step(910, 720, 60);\n\
         \x20   let step4 = control_step(9999, 500, 80);\n\
         \x20   return step1 + step2 + step3 + step4;\n\
         }\n\
         println(run());\n",
    )
    .unwrap();

    let output = Command::new(bin())
        .arg(&src)
        .output()
        .expect("spawn rz (interpreter)");
    assert_eq!(
        output.status.code(),
        Some(0),
        "interpreter run should succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout.lines().next(), Some("180"));

    let _ = std::fs::remove_dir_all(&dir);
}
