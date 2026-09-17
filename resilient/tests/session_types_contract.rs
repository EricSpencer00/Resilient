//! End-to-end coverage for RES-4268's session protocol call-site checks.

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
        "res_session_types_contract_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write session types fixture");
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
#[session(protocol = "send(int).recv(bool).close")]
fn ch() -> int { return 0; }
"#;

#[test]
fn calls_beyond_protocol_are_rejected_at_call_site() {
    let output = run_strict(&format!(
        "{HEADER}\nfn main() {{\n    println(ch());\n    println(ch());\n    println(ch());\n    println(ch());\n}}\nmain();\n"
    ));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(
        stderr.contains("session protocol `ch` already terminated step 3"),
        "unexpected diagnostic: {stderr}"
    );
    assert!(stderr.contains(":9:"), "missing call-site line: {stderr}");
}

#[test]
fn exact_protocol_length_still_runs() {
    let output = run_strict(&format!(
        "{HEADER}\nfn main() {{\n    println(ch());\n    println(ch());\n    println(ch());\n}}\nmain();\n"
    ));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "exact protocol sequence failed:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("Program executed successfully"),
        "successful program did not run: {stdout}"
    );
}

#[test]
fn branch_calls_remain_conservative() {
    let output = run_strict(&format!(
        "{HEADER}\nfn main() {{\n    if false {{ println(ch()); }}\n    println(ch());\n    println(ch());\n    println(ch());\n}}\nmain();\n"
    ));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "branch call should not affect straight-line analysis:\nstdout={stdout}\nstderr={stderr}"
    );
}
