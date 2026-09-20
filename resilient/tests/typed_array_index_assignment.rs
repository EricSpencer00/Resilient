//! Regression coverage for RES-4474: typed array writes preserve element contracts.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_typed_array_index_assignment_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write typed-array fixture");
    path
}

fn typecheck(source: &str) -> Output {
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
fn typed_array_index_writes_require_the_element_type() {
    let output = typecheck(
        "fn main() {
             let ints: array<int> = [1, 2];
             ints[0] = \"wrong\";
         }
         main();
        ",
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "array<int> accepted a string indexed write: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("indexed element"),
        "diagnostic did not identify the typed element contract: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn typed_array_index_writes_accept_matching_and_any_values() {
    let output = typecheck(
        "fn main() {
             let ints: array<int> = [1, 2];
             ints[0] = 3;
             let strings: array<string> = [\"a\", \"b\"];
             strings[1] = \"c\";
             let dynamic: array<any> = [1, 2];
             dynamic[0] = \"allowed\";
         }
         main();
        ",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "compatible typed-array writes were rejected: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn string_array_index_writes_reject_integer_values() {
    let output = typecheck(
        "fn main() {
             let strings: array<string> = [\"a\"];
             strings[0] = 7;
         }
         main();
        ",
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "array<string> accepted an integer indexed write: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
