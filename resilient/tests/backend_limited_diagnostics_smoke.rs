//! RES-3153: pin backend-limited diagnostics for verifier-only surfaces.

#![cfg(not(feature = "z3"))]

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_file(tag: &str, body: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_3153_backend_limited_{}_{}_{}.rz",
        tag,
        std::process::id(),
        n
    ));
    std::fs::write(&path, body).expect("write scratch source");
    path
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "res_3153_backend_limited_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ))
}

#[test]
fn verify_all_requires_z3_feature_in_default_build() {
    let cert_dir = tmp_dir("verify_all");
    let output = Command::new(bin())
        .args(["verify-all"])
        .arg(&cert_dir)
        .output()
        .expect("spawn rz verify-all");

    assert_eq!(
        output.status.code(),
        Some(1),
        "verify-all should fail before pretending the verifier surface is available; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Backend-limited: rz verify-all requires the `z3` feature")
            && stderr.contains("cargo build --features z3")
            && stderr.contains("Stable path:"),
        "verify-all should explain the backend limit and stable path; got:\n{stderr}"
    );
}

#[test]
fn emit_certificate_requires_z3_feature_in_default_build() {
    let src = tmp_file("emit_cert", "fn main() { return 1; }\nmain();\n");
    let cert_dir = tmp_dir("certs");
    let output = Command::new(bin())
        .args(["--seed", "0", "--emit-certificate"])
        .arg(&cert_dir)
        .arg(&src)
        .output()
        .expect("spawn rz --emit-certificate");
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_dir_all(&cert_dir);

    assert_eq!(
        output.status.code(),
        Some(1),
        "--emit-certificate should fail when z3 is not compiled in; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Backend-limited: --emit-certificate requires the `z3` feature")
            && stderr.contains("cargo build --features z3")
            && stderr.contains("Stable path:"),
        "--emit-certificate should explain the backend limit and stable path; got:\n{stderr}"
    );
}

#[test]
fn z3_theory_requires_z3_feature_in_default_build() {
    let src = tmp_file("z3_theory", "fn main() { return 1; }\nmain();\n");
    let output = Command::new(bin())
        .args(["check", "--z3-theory=bv"])
        .arg(&src)
        .output()
        .expect("spawn rz check --z3-theory");
    let _ = std::fs::remove_file(&src);

    assert_eq!(
        output.status.code(),
        Some(2),
        "--z3-theory should be rejected as a z3-only flag; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Backend-limited: --z3-theory requires the `z3` feature")
            && stderr.contains("cargo build --features z3")
            && stderr.contains("Stable path:"),
        "--z3-theory should explain the backend limit and stable path; got:\n{stderr}"
    );
}
