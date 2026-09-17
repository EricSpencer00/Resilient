//! RES-4269: end-to-end typestate call-site enforcement.
//!
//! These cases invoke the strict CLI so they exercise attribute collection,
//! typechecking, and the extension pass together.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_typestate_enforcement_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write typestate fixture");
    path
}

fn run_strict(source: &str) -> Output {
    let path = scratch_file(source);
    let output = Command::new(bin())
        .args(["--typecheck-strict"])
        .arg(&path)
        .output()
        .expect("spawn rz --typecheck-strict");
    let _ = std::fs::remove_file(path);
    output
}

const HEADER: &str = r#"
#[typestate(states = "Closed Open", transitions = "Closed:open->Open Open:close->Closed")]
struct File { int fd }

fn open(File f) -> int { return f.fd; }
fn close(File f) -> int { return 0; }
"#;

#[test]
fn illegal_transition_is_rejected_at_call_site() {
    let output = run_strict(&format!(
        "{HEADER}\nfn main() {{\n    let f = new File {{ fd: 3 }};\n    println(close(f));\n}}\nmain();\n"
    ));
    assert_eq!(
        output.status.code(),
        Some(1),
        "closed-file close should fail: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("typestate violation"),
        "unexpected: {stderr}"
    );
    assert!(stderr.contains("File"), "unexpected: {stderr}");
    assert!(stderr.contains("Closed"), "unexpected: {stderr}");
    assert!(stderr.contains("close"), "unexpected: {stderr}");
    assert!(
        stderr.contains(":10:"),
        "diagnostic lost the call-site line: {stderr}"
    );
}

#[test]
fn legal_transition_sequence_still_runs() {
    let output = run_strict(&format!(
        "{HEADER}\nfn main() {{\n    let f = new File {{ fd: 3 }};\n    println(open(f));\n    println(close(f));\n}}\nmain();\n"
    ));
    assert_eq!(
        output.status.code(),
        Some(0),
        "open then close should pass: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Program executed successfully"),
        "successful program did not run: stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn typestate_values_passed_to_unknown_functions_remain_permissive() {
    let output = run_strict(&format!(
        "{HEADER}\nfn inspect(File f) -> int {{ return f.fd; }}\nfn main() {{\n    let f = new File {{ fd: 3 }};\n    println(inspect(f));\n}}\nmain();\n"
    ));
    assert_eq!(
        output.status.code(),
        Some(0),
        "unknown function boundary should remain permissive: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}
