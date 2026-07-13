//! RES-3840: smoke tests for `rz --vibe-gate <threshold>`.
//!
//! Verifies that --vibe-gate:
//! - exits 0 when vibe_debt score <= threshold
//! - exits 2 when vibe_debt score > threshold
//! - emits structured JSON to stderr
//! - rejects invalid threshold values gracefully (exit 2, no panic)

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
        "res_vibe_gate_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

#[test]
fn vibe_gate_passes_generous_threshold() {
    // A simple program with good contracts should pass a generous threshold
    let dir = tmp_dir("pass");
    let src_path = dir.join("good.rz");
    let src = r#"
fn add(int x, int y) // @pure
  // @requires x >= 0 && y >= 0
  // @ensures return >= 0
  -> int {
    return x + y;
}
fn main() {
    print(add(1, 2));
}
main();
"#;
    std::fs::write(&src_path, src).unwrap();

    let output = Command::new(bin())
        .args(["--vibe-gate", "0.8", &src_path.to_string_lossy()])
        .output()
        .expect("spawn rz --vibe-gate");

    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0 for generous threshold; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#""passed": true"#),
        "expected 'passed': true in JSON; stderr={stderr}"
    );
    assert!(
        stderr.contains(r#""vibe_debt":"#),
        "expected vibe_debt in JSON; stderr={stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn vibe_gate_fails_strict_threshold() {
    // A program without any contracts should fail a very strict threshold
    let dir = tmp_dir("fail");
    let src_path = dir.join("uncontracted.rz");
    let src = r#"
fn add(int x, int y) {
    return x + y;
}
fn main() {
    print(add(1, 2));
}
main();
"#;
    std::fs::write(&src_path, src).unwrap();

    let output = Command::new(bin())
        .args(["--vibe-gate", "0.01", &src_path.to_string_lossy()])
        .output()
        .expect("spawn rz --vibe-gate");

    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit 2 for failing threshold; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#""passed": false"#),
        "expected 'passed': false in JSON; stderr={stderr}"
    );
    assert!(
        stderr.contains(r#""vibe_debt":"#),
        "expected vibe_debt in JSON; stderr={stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn vibe_gate_rejects_invalid_threshold() {
    let dir = tmp_dir("invalid");
    let src_path = dir.join("any.rz");
    std::fs::write(&src_path, "fn main() { }").unwrap();

    let output = Command::new(bin())
        .args(["--vibe-gate", "invalid", &src_path.to_string_lossy()])
        .output()
        .expect("spawn rz --vibe-gate invalid");

    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit 2 for invalid threshold; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error:") && stderr.contains("--vibe-gate"),
        "expected clean error message; stderr={stderr}"
    );
    // Verify no Rust panic trace appears
    assert!(
        !stderr.contains("panicked") && !stderr.contains("thread 'main'"),
        "expected no panic output; stderr={stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn vibe_gate_rejects_out_of_range_threshold() {
    let dir = tmp_dir("out_of_range");
    let src_path = dir.join("any.rz");
    std::fs::write(&src_path, "fn main() { }").unwrap();

    // Test threshold > 1.0
    let output = Command::new(bin())
        .args(["--vibe-gate", "1.5", &src_path.to_string_lossy()])
        .output()
        .expect("spawn rz --vibe-gate 1.5");

    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit 2 for threshold > 1.0; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error:") && stderr.contains("--vibe-gate"),
        "expected clean error message; stderr={stderr}"
    );

    // Test threshold < 0.0
    let output = Command::new(bin())
        .args(["--vibe-gate", "-0.5", &src_path.to_string_lossy()])
        .output()
        .expect("spawn rz --vibe-gate -0.5");

    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit 2 for threshold < 0.0; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn vibe_gate_with_equals_form() {
    // Test the --vibe-gate=<threshold> form
    let dir = tmp_dir("equals_form");
    let src_path = dir.join("good.rz");
    let src = r#"
fn add(int x, int y) // @pure
  // @requires x >= 0
  // @ensures return >= 0
  -> int {
    return x + y;
}
fn main() {
    print(add(1, 2));
}
main();
"#;
    std::fs::write(&src_path, src).unwrap();

    let output = Command::new(bin())
        .args([
            "--vibe-gate=0.8".to_string(),
            src_path.to_string_lossy().to_string(),
        ])
        .output()
        .expect("spawn rz --vibe-gate=");

    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#""passed": true"#),
        "expected passed:true in JSON; stderr={stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
