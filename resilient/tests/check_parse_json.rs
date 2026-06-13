//! RES-3382: parse errors in `rz check --emit-diagnostics-json`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_file(tag: &str, body: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("res_3382_{}_{}_{}.rz", tag, std::process::id(), n));
    std::fs::write(&path, body).expect("write scratch");
    path
}

#[test]
fn check_emit_diagnostics_json_reports_parse_errors_as_json_only() {
    let src = tmp_file("json_parse_error", "fn main( {\n");

    let out = Command::new(bin())
        .args(["--", "check", "--emit-diagnostics-json"])
        .arg(&src)
        .output()
        .expect("spawn check --emit-diagnostics-json parse error");

    assert_eq!(
        out.status.code(),
        Some(1),
        "expected parse failure exit 1, got {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("\x1b["),
        "stdout must not contain ANSI escapes; stdout={stdout:?}"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("expected valid diagnostics JSON array");
    let arr = parsed
        .as_array()
        .expect("expected diagnostics JSON top level array");
    assert!(
        !arr.is_empty(),
        "expected at least one parse diagnostic, got: {stdout}"
    );
    for diag in arr {
        assert_eq!(diag.get("severity").and_then(|v| v.as_str()), Some("error"));
        assert_eq!(diag.get("code").and_then(|v| v.as_str()), Some("parse"));
        assert!(
            diag.get("line").and_then(|v| v.as_u64()).is_some(),
            "diagnostic missing numeric line: {diag}"
        );
        assert!(
            diag.get("column").and_then(|v| v.as_u64()).is_some(),
            "diagnostic missing numeric column: {diag}"
        );
        assert!(
            diag.get("message")
                .and_then(|v| v.as_str())
                .is_some_and(|msg| !msg.is_empty()),
            "diagnostic missing message: {diag}"
        );
    }

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.trim().is_empty(),
        "stderr must not contain parser human formatting; stderr={stderr:?}"
    );
}
