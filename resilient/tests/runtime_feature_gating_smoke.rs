//! RES-3136: smoke tests for runtime feature-gating contracts.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn cargo_bin() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

fn tmp_target_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_runtime_gate_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&path).expect("create target dir");
    path
}

fn runtime_manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("resilient crate has repo parent")
        .join("resilient-runtime/Cargo.toml")
}

#[test]
fn runtime_rejects_alloc_static_only_feature_mix() {
    let target_dir = tmp_target_dir("alloc_static_only");
    let output = Command::new(cargo_bin())
        .arg("check")
        .arg("--manifest-path")
        .arg(runtime_manifest())
        .arg("--features")
        .arg("alloc static-only")
        .arg("--target-dir")
        .arg(&target_dir)
        .output()
        .expect("spawn cargo check for runtime feature mix");

    let _ = std::fs::remove_dir_all(&target_dir);

    assert!(
        !output.status.success(),
        "alloc + static-only must fail closed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("`alloc` and `static-only` are mutually exclusive"),
        "expected explicit runtime feature-gating error; stderr={stderr}"
    );
}
