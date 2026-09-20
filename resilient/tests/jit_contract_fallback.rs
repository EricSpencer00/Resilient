//! RES-4525: contract-bearing programs must use the VM's runtime checks.

#![cfg(feature = "jit")]

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_jit_contract_fallback_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write JIT contract fixture");
    path
}

fn run(mode: &str, source: &str) -> Output {
    let path = scratch_file(source);
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "--no-typecheck", "--verbose", mode])
        .arg(&path)
        .output()
        .expect("spawn rz backend");
    let _ = std::fs::remove_file(path);
    output
}

#[test]
fn violated_ensures_matches_vm_and_reports_precompile_fallback() {
    let source = include_str!("fixtures/jit_contract_fallback_gap.rz");
    let vm = run("--vm", source);
    let jit = run("--jit", source);

    for (backend, output) in [("VM", &vm), ("JIT", &jit)] {
        assert_eq!(
            output.status.code(),
            Some(1),
            "{backend} should reject the violated ensures: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("Contract violation in fn broken: ensures result == x failed"),
            "{backend} lost the runtime contract diagnostic: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let jit_stderr = String::from_utf8_lossy(&jit.stderr);
    assert!(
        jit_stderr.contains("fell back to the VM")
            && jit_stderr.contains("runtime contracts require VM enforcement"),
        "JIT should report its precompile-safe contract fallback: {jit_stderr}"
    );
    assert!(
        !String::from_utf8_lossy(&jit.stdout).contains("Program executed successfully"),
        "JIT must not report success after a contract violation"
    );
}

#[test]
fn requires_and_recovers_to_also_force_vm_fallback() {
    let cases = [
        (
            "requires",
            "fn guarded(int x) -> int requires x > 0 { return x; } return guarded(0);",
        ),
        (
            "recovers_to",
            "fn broken(int x) -> int recovers_to: result == 0; { return 1; } return broken(1);",
        ),
    ];

    for (kind, source) in cases {
        let output = run("--jit", source);
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("fell back to the VM")
                && String::from_utf8_lossy(&output.stderr)
                    .contains("runtime contracts require VM enforcement"),
            "{kind} contract did not take the guarded fallback: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn contract_free_program_remains_native() {
    let output = run("--jit", "return 1 + 1;");
    assert_eq!(
        output.status.code(),
        Some(0),
        "contract-free JIT program failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("2\n"));
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("fell back to the VM"),
        "contract-free JIT program unexpectedly fell back: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
