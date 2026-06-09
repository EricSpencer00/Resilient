//! RES-3128: integration tests for `--typecheck-strict`.
//!
//! The default driver keeps type errors soft so it can still execute
//! the program, but `--typecheck-strict` must flip that behavior into
//! a fatal compile-time failure.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_file(tag: &str, body: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_typecheck_strict_{}_{}_{}.rz",
        tag,
        std::process::id(),
        n
    ));
    std::fs::write(&path, body).expect("write scratch file");
    path
}

#[test]
fn default_driver_keeps_type_errors_soft() {
    let path = tmp_file(
        "soft",
        "fn main() {\n    let x: Int = \"not an int\";\n    return 0;\n}\nmain();\n",
    );
    let output = Command::new(bin()).arg(&path).output().expect("spawn rz");

    let _ = std::fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(0),
        "default driver should stay soft on type errors; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("Program executed successfully"),
        "default driver should still reach program completion; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stderr.contains("Type error:"),
        "default driver should surface the type error as a diagnostic; stderr={stderr}"
    );
}

#[test]
fn typecheck_strict_turns_type_errors_fatal() {
    let path = tmp_file(
        "strict",
        "fn main() {\n    let x: Int = \"not an int\";\n    return 0;\n}\nmain();\n",
    );
    let output = Command::new(bin())
        .args(["--typecheck-strict"])
        .arg(&path)
        .output()
        .expect("spawn rz --typecheck-strict");

    let _ = std::fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(1),
        "strict typecheck should fail fast on type errors; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error: Type check failed"),
        "strict mode should report a fatal typecheck failure; stderr={stderr}"
    );
}
