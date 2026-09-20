//! RES-4568: call-site columns must follow peephole PC rewrites.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_stacktrace_optimizer_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write stacktrace optimizer fixture");
    path
}

fn run_vm(source: &str) -> Output {
    let path = scratch_file(source);
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg("--vm")
        .arg(&path)
        .output()
        .expect("spawn rz stacktrace optimizer fixture");
    let _ = std::fs::remove_file(path);
    output
}

#[test]
fn vm_stacktrace_preserves_call_column_after_peephole_fold() {
    let source = r#"
fn leaf() {
    let trace = stacktrace()
    println(trace[1])
}

fn caller() {
    let folded = 1 + 0
    leaf()
}

caller()
"#;

    let output = run_vm(source);
    assert!(
        output.status.success(),
        "VM execution failed: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(":9:10"),
        "stacktrace lost the call-site column after peephole folding: {stdout:?}"
    );
}
