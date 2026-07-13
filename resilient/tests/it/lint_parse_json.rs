//! RES-3383: parse errors in `rz lint --emit-diagnostics-json`.

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
        std::env::temp_dir().join(format!("res_3383_{}_{}_{}.rz", tag, std::process::id(), n));
    std::fs::write(&path, body).expect("write scratch");
    path
}

#[test]
fn lint_emit_diagnostics_json_reports_parse_errors_as_json() {
    let src = tmp_file("json_parse_error", "fn main( {\n");
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .arg("--emit-diagnostics-json")
        .output()
        .expect("spawn lint --emit-diagnostics-json parse error");

    assert_eq!(
        out.status.code(),
        Some(2),
        "expected parse failure exit 2, got {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
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
        assert!(diag.get("line").and_then(|v| v.as_u64()).is_some());
        assert!(diag.get("column").and_then(|v| v.as_u64()).is_some());
        assert!(diag.get("message").and_then(|v| v.as_str()).is_some());
        assert_eq!(
            diag.get("file").and_then(|v| v.as_str()),
            Some(src.to_string_lossy().as_ref())
        );
    }

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("Parser error") && !stderr.contains("lint aborted due parse errors"),
        "expected no human parser text in JSON mode, got stderr: {stderr}"
    );

    let _ = std::fs::remove_file(&src);
}
