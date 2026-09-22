//! RES-4605: reject statically non-iterable quantifier sources.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_quantifier_iterable_typecheck_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write quantifier fixture");
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
fn scalar_quantifier_source_is_rejected_before_execution() {
    let output = run_strict("let result = exists item in 42: true;\nprintln(result);\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(
        stderr
            .contains("quantifier iterable source must be array, bytes, or dynamic value, got int"),
        "missing iterable-source diagnostic: {stderr}"
    );
    assert!(
        stderr.contains(":1:"),
        "diagnostic lost source position: {stderr}"
    );
}

#[test]
fn arrays_and_bytes_remain_valid_quantifier_sources() {
    let output = run_strict(
        "let from_array = exists item in [1, 2, 3]: item == 2;\n\
         let from_bytes = exists byte in b\"abc\": byte == 98;\n\
         println(from_array);\n\
         println(from_bytes);\n",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "iterable quantifier sources failed: stdout={stdout} stderr={stderr}"
    );
    assert!(stdout.contains("true"), "unexpected output: {stdout}");
    assert!(
        stderr
            .lines()
            .all(|line| line.is_empty() || line.starts_with("seed=")),
        "unexpected diagnostics: {stderr}"
    );
}
