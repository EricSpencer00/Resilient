//! RES-4554: VM and interpreter must enforce anonymous function postconditions.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "res_anonymous_contract_postcheck_{}_{}.rz",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, source).expect("write anonymous contract regression source");
    path
}

fn run(source: &str, vm: bool) -> Output {
    let path = scratch_file(source);
    let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
    command.args(["--no-cache", "--no-typecheck"]);
    if vm {
        command.arg("--vm");
    }
    let output = command
        .arg(&path)
        .output()
        .expect("run anonymous contract regression source");
    let _ = fs::remove_file(path);
    output
}

#[test]
fn anonymous_function_postconditions_match_interpreter_and_vm() {
    let cases = [
        (
            "violated ensures",
            "let broken = fn(int x) -> int ensures result > x { return x; }; return broken(1);",
            false,
            "ensures",
        ),
        (
            "satisfied ensures with captured closure",
            "let offset = 10; let checked = fn(int x) -> int ensures result > x { return x + offset; }; return checked(1);",
            true,
            "",
        ),
        (
            "violated recovers_to",
            "let broken = fn(int x) -> int recovers_to: result == 0; { return x; }; return broken(1);",
            false,
            "recovers_to",
        ),
    ];

    for (name, source, succeeds, contract) in cases {
        for vm in [false, true] {
            let output = run(source, vm);
            let backend = if vm { "VM" } else { "interpreter" };
            assert_eq!(
                output.status.success(),
                succeeds,
                "{backend} result for {name} was unexpected: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            if !succeeds {
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(
                    stderr.contains("Contract violation in fn <anon>") && stderr.contains(contract),
                    "{backend} lost the {contract} diagnostic for {name}: {stderr}"
                );
            }
        }
    }
}
