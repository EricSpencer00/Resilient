//! RES-2991: smoke test for `rz bench --summary-json`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_bench_cli_summary_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    fs::create_dir_all(&p).expect("mkdir");
    p
}

#[test]
fn bench_writes_machine_readable_summary_artifact() {
    let dir = tmp_dir("summary_json");
    let summary_path = dir.join("bench-summary.json");

    let output = Command::new(bin())
        .args([
            "bench",
            "examples/bench_simple.rz",
            "--summary-json",
            summary_path.to_str().expect("summary path utf8"),
        ])
        .output()
        .expect("spawn resilient bench");

    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("summary.benchmarks="),
        "expected stable summary footer; stdout={stdout}"
    );
    assert!(
        stdout.contains("artifact.summary_json="),
        "expected artifact location footer; stdout={stdout}"
    );
    assert!(
        stdout.contains(summary_path.to_str().expect("summary path utf8")),
        "expected emitted summary path; stdout={stdout}"
    );

    let text = fs::read_to_string(&summary_path).expect("read summary json");
    let json: serde_json::Value = serde_json::from_str(&text).expect("parse summary json");
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["source"], "examples/bench_simple.rz");
    assert_eq!(json["warmup_iters"], 1);
    assert_eq!(json["run_iters"], 3);
    assert_eq!(json["benchmark_count"], 5);

    let benches = json["benchmarks"].as_array().expect("benchmarks array");
    assert_eq!(benches.len(), 5);
    let first = &benches[0];
    assert_eq!(first["name"], "empty block");
    assert!(first.get("mean_ns").is_some(), "missing mean_ns: {first:?}");
    assert!(
        first.get("baseline_mean_ns").is_some(),
        "missing baseline_mean_ns field: {first:?}"
    );
    assert!(
        first.get("delta_pct").is_some(),
        "missing delta_pct field: {first:?}"
    );
}
