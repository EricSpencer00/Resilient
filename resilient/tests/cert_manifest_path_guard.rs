//! Regression coverage for manifest-controlled certificate paths.

#![cfg(feature = "z3")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("res_4768_{}_{}_{}", tag, std::process::id(), n));
    std::fs::create_dir_all(&path).expect("create temporary certificate directory");
    path
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_manifest(dir: &Path, cert: &str, sha256: &str) {
    let manifest = format!(
        r#"{{
  "program": "test.rs",
  "obligations": [
    {{"fn": "foo", "kind": "ensures", "idx": 0,
      "cert": "{cert}", "sha256": "{sha256}"}}
  ]
}}"#
    );
    std::fs::write(dir.join("manifest.json"), manifest).expect("write manifest");
}

fn run_verify_all(dir: &Path) -> Output {
    Command::new(bin())
        .args(["verify-all"])
        .arg(dir)
        .output()
        .expect("spawn verify-all")
}

#[test]
fn rejects_certificate_paths_that_leave_manifest_directory() {
    for (tag, cert) in [
        ("parent", "../outside.smt2"),
        ("nested", "nested/inside.smt2"),
        ("backslash", r"..\outside.smt2"),
        ("absolute", "/tmp/outside.smt2"),
        ("drive", r"C:\outside.smt2"),
    ] {
        let dir = temp_dir(tag);
        write_manifest(&dir, cert, &"00".repeat(32));

        let output = run_verify_all(&dir);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "path case should be rejected: {tag}"
        );
        assert!(
            stderr.contains("invalid `cert`") || stderr.contains("invalid cert"),
            "unexpected diagnostics for path case {tag}"
        );

        std::fs::remove_dir_all(&dir).expect("remove temporary certificate directory");
    }
}

#[test]
fn accepts_a_single_relative_smt2_filename() {
    let dir = temp_dir("valid");
    let cert = b"(assert true)\n(check-sat)\n";
    let cert_name = "foo__ensures__0.smt2";
    std::fs::write(dir.join(cert_name), cert).expect("write certificate");
    write_manifest(&dir, cert_name, &sha256_hex(cert));

    let output = run_verify_all(&dir);
    assert!(
        output.status.success(),
        "valid certificate path failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("all checks passed"));

    std::fs::remove_dir_all(&dir).expect("remove temporary certificate directory");
}
