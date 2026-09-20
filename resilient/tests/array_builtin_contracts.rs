//! Compile-time contracts for builtins whose first argument is an array.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_array_builtin_contracts_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write array builtin contract fixture");
    path
}

fn run_strict(source: &str) -> Output {
    let path = scratch_file(source);
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--typecheck-strict"])
        .arg(&path)
        .output()
        .expect("spawn rz --typecheck-strict");
    let _ = std::fs::remove_file(path);
    output
}

#[test]
fn array_builtin_contracts_accept_arrays_and_dynamic_values() {
    let output = run_strict(
        "gcd_array([1, 2, 3]);
         lcm_array([2, 4, 8]);
         array_position([\"needle\"], \"needle\", 0);
         array_pad_left([1], 2, \"fill\");
         array_pad_right([true], 2, 42);
         array_swap([1, 2], 0, 1);
         array_insert_at([1], 0, \"value\");
         array_remove_at([1], 0);
         array_set_at([1], 0, false);
        ",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "valid array builtin calls failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn array_builtin_contracts_reject_non_array_first_arguments() {
    let cases = [
        ("gcd_array", "gcd_array(1);"),
        ("lcm_array", "lcm_array(1);"),
        ("array_position", "array_position(1, 0, 0);"),
        ("array_pad_left", "array_pad_left(1, 2, 0);"),
        ("array_pad_right", "array_pad_right(1, 2, 0);"),
        ("array_swap", "array_swap(1, 0, 0);"),
        ("array_insert_at", "array_insert_at(1, 0, 0);"),
        ("array_remove_at", "array_remove_at(1, 0);"),
        ("array_set_at", "array_set_at(1, 0, 0);"),
    ];

    for (builtin, call) in cases {
        let output = run_strict(call);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{builtin} accepted a scalar first argument: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn array_builtin_contracts_reject_struct_first_arguments() {
    let output = run_strict(
        "struct Sample { int value }
         let sample = new Sample { value: 1 };
         array_remove_at(sample, 0);
        ",
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "array builtin accepted a struct first argument: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
