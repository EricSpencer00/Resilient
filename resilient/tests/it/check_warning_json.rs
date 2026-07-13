//! RES-3386: warning-producing check passes use JSON diagnostics.

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
        std::env::temp_dir().join(format!("res_3386_{}_{}_{}.rz", tag, std::process::id(), n));
    std::fs::write(&path, body).expect("write scratch");
    path
}

#[test]
fn check_emit_diagnostics_json_reports_warning_passes_without_stderr() {
    let src = tmp_file(
        "mutation_warning",
        "fn mutate(int x) -> int { return x + 1; }\n",
    );

    let out = Command::new(bin())
        .args(["check", "--emit-diagnostics-json"])
        .arg(&src)
        .output()
        .expect("spawn check --emit-diagnostics-json warning case");

    assert_eq!(
        out.status.code(),
        Some(0),
        "expected clean check exit 0, got {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("mutation:")
            && !stderr.contains("warning[mutation]")
            && !stderr.contains("warning[vibe_debt]")
            && !stderr.contains("warning[resilience]"),
        "JSON diagnostics mode must not print warning passes to stderr; stderr={stderr:?}"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("expected valid diagnostics JSON array");
    let arr = parsed
        .as_array()
        .expect("expected diagnostics JSON top level array");

    assert!(
        arr.iter().any(|diag| {
            diag.get("severity").and_then(|v| v.as_str()) == Some("warning")
                && diag.get("code").and_then(|v| v.as_str()) == Some("mutation")
        }),
        "expected a mutation warning diagnostic in JSON output, got: {stdout}"
    );
}
