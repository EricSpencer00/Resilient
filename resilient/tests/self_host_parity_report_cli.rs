use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn temp_path(stem: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "res2992_{}_{}_{}.{}",
        std::process::id(),
        stem,
        nanos,
        ext
    ))
}

#[test]
fn self_host_parity_report_publishes_gap_artifact() {
    let report_path = temp_path("self-host-parity-report", "json");
    let output = Command::new(bin())
        .arg("self-host-parity-report")
        .arg("--json-out")
        .arg(&report_path)
        .output()
        .expect("spawn self-host-parity-report");
    assert!(
        output.status.success(),
        "self-host-parity-report failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("self-host parity report"));
    assert!(stdout.contains("artifacts.token_parity=pass"));
    assert!(stdout.contains("artifacts.ast_parity=pass"));
    assert!(stdout.contains("artifacts.parse_error_location=pass"));
    assert!(stdout.contains("features.covered="));
    assert!(stdout.contains("features.missing="));
    assert!(stdout.contains("features.divergent=0"));
    assert!(stdout.contains("stmt.if_else"));
    assert!(stdout.contains("expr.float_literal"));
    assert!(stdout.contains(&report_path.display().to_string()));

    let report_text = fs::read_to_string(&report_path).expect("read JSON report");
    let report: Value = serde_json::from_str(&report_text).expect("parse JSON report");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["success_case_count"], 3);
    assert_eq!(report["error_case_count"], 1);
    assert_eq!(report["artifact_summary"]["token_parity"], "pass");
    assert_eq!(report["artifact_summary"]["ast_parity"], "pass");
    assert_eq!(report["artifact_summary"]["parse_error_location"], "pass");
    assert_eq!(report["feature_summary"]["divergent"], 0);
    assert!(
        report["feature_summary"]["covered"]
            .as_u64()
            .expect("covered count")
            > 0
    );

    let features = report["features"].as_array().expect("features array");
    assert!(
        features
            .iter()
            .any(|feature| feature["id"] == "stmt.if_else" && feature["status"] == "covered")
    );
    assert!(
        features
            .iter()
            .any(|feature| feature["id"] == "expr.float_literal" && feature["status"] == "missing")
    );

    let _ = fs::remove_file(&report_path);
}
