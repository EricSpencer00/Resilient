//! RES-390: smoke tests for the distributed-invariant verifier.
//!
//! Exercises `resilient check` against the two golden examples:
//!
//! - `examples/cluster_single_leader_ok.rz` — step-down handler
//!   preserves the single-leader invariant; `check` must exit 0.
//! - `examples/cluster_single_leader_bad.rz` — unconditional
//!   `become_leader` breaks the invariant; `check` must exit 1
//!   with a `cluster-invariant error` diagnostic.
//!
//! The tests only run under `--features z3` because the cluster
//! verifier itself is Z3-gated. On builds without Z3, `check` is
//! expected to exit 0 for both files (parser + typechecker accept
//! them); the happy-path assertion is still useful in that mode.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name)
}

#[test]
fn cluster_single_leader_ok_passes_check() {
    let output = Command::new(bin())
        .arg("check")
        .arg(example("cluster_single_leader_ok.rz"))
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected check to pass on ok cluster; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(feature = "z3")]
#[test]
fn cluster_single_leader_bad_fails_check() {
    let output = Command::new(bin())
        .arg("check")
        .arg(example("cluster_single_leader_bad.rz"))
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected check to fail on broken cluster; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cluster-invariant error"),
        "expected cluster-invariant diagnostic; stderr={stderr}"
    );
    assert!(
        stderr.contains("become_leader"),
        "expected handler name `become_leader` in diagnostic; stderr={stderr}"
    );
}
