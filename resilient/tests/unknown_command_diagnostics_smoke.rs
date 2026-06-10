//! RES-3159: distinguish command typos from missing source files.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "res_3159_unknown_command_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn bare_unknown_token_reports_command_or_file_hint() {
    let output = Command::new(bin())
        .arg("frobnicate")
        .output()
        .expect("spawn rz frobnicate");

    assert_eq!(
        output.status.code(),
        Some(2),
        "unknown command-like token should be a usage error; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error: unknown command or file `frobnicate`")
            && stderr.contains("rz --help")
            && !stderr.contains("Error reading file")
            && !stderr.contains("seed="),
        "unknown command diagnostic should be focused; got:\n{stderr}"
    );
}

#[test]
fn existing_extensionless_relative_file_still_executes() {
    let dir = tmp_dir("relative_file");
    let src = dir.join("program");
    std::fs::write(&src, "println(\"res3159 relative file ok\");\n").expect("write program");

    let output = Command::new(bin())
        .current_dir(&dir)
        .args(["--seed", "0", "program"])
        .output()
        .expect("spawn rz program");
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        output.status.code(),
        Some(0),
        "existing relative file should still execute; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("res3159 relative file ok"),
        "expected program output from extensionless relative file; got:\n{stdout}"
    );
}
