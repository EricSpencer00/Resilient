//! RES-195: integration tests for `resilient verify-all <dir>`.
//!
//! Without `--features z3` in the test build, the typechecker
//! doesn't produce any real `.smt2` obligations, so these tests
//! hand-craft a plausible cert directory (manifest.json +
//! `.smt2` payload + optional signature) and drive the binary
//! against it. That gives clean coverage of the manifest parser,
//! the sha256 check, the per-obligation signature check, and
//! the error paths — without depending on libz3 being present
//! on the test host.
//!
//! The signed variants use an ephemeral Ed25519 keypair; the
//! `--pubkey` override on `verify-all` lets us supply the
//! matching public key.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use ed25519_dalek::SigningKey;
use ed25519_dalek::ed25519::signature::Signer;
use rand_core::OsRng;
use sha2::{Digest, Sha256};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("res_195_{}_{}_{}", tag, std::process::id(), n));
    std::fs::create_dir_all(&p).expect("mkdir tmp");
    p
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(*b >> 4) as usize] as char);
        out.push(HEX[(*b & 0x0f) as usize] as char);
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex_encode(&h.finalize())
}

fn write_pubkey_pem(dir: &std::path::Path, pub_bytes: &[u8; 32]) -> PathBuf {
    let pem = format!(
        "-----BEGIN ED25519 PUBLIC KEY-----\n{}\n-----END ED25519 PUBLIC KEY-----\n",
        hex_encode(pub_bytes)
    );
    let p = dir.join("pub.pem");
    std::fs::write(&p, pem).unwrap();
    p
}

/// Write one cert file + the manifest that references it.
/// `sig_key` lets the caller control whether the obligation is
/// signed + whether the signature matches the cert contents.
fn write_cert_dir_with_one_obligation(
    dir: &std::path::Path,
    cert_filename: &str,
    cert_bytes: &[u8],
    sig_hex: Option<&str>,
    sha_override: Option<&str>,
) {
    std::fs::write(dir.join(cert_filename), cert_bytes).unwrap();
    let sha = sha_override
        .map(String::from)
        .unwrap_or_else(|| sha256_hex(cert_bytes));
    let sig_field = match sig_hex {
        Some(s) => format!(r#", "sig": "{}""#, s),
        None => String::new(),
    };
    let manifest = format!(
        r#"{{
  "program": "test.rs",
  "obligations": [
    {{"fn": "foo", "kind": "ensures", "idx": 0,
      "cert": "{cert_filename}", "sha256": "{sha}"{sig_field}}}
  ]
}}"#
    );
    std::fs::write(dir.join("manifest.json"), manifest).unwrap();
}

#[test]
fn verify_all_happy_path_unsigned() {
    let dir = tmp_dir("unsigned");
    write_cert_dir_with_one_obligation(
        &dir,
        "foo__ensures__0.smt2",
        b"(assert (> x 0))\n(check-sat)\n",
        None,
        None,
    );

    let out = Command::new(bin())
        .args(["verify-all"])
        .arg(&dir)
        .output()
        .expect("spawn verify-all");
    assert!(
        out.status.success(),
        "expected 0, got {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("all checks passed"), "stdout was: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_all_detects_sha256_mismatch() {
    let dir = tmp_dir("bad_sha");
    write_cert_dir_with_one_obligation(
        &dir,
        "foo__ensures__0.smt2",
        b"real bytes",
        None,
        Some("deadbeef".repeat(8).as_str()), // wrong 32-byte hash
    );

    let out = Command::new(bin())
        .args(["verify-all"])
        .arg(&dir)
        .output()
        .expect("spawn");
    assert!(!out.status.success(), "should fail on sha mismatch");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("FAILED") || String::from_utf8_lossy(&out.stdout).contains("FAIL"),
        "stderr: {stderr}\nstdout: {}",
        String::from_utf8_lossy(&out.stdout),
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_all_happy_path_signed() {
    let dir = tmp_dir("signed");
    let cert_bytes = b"(assert true)\n(check-sat)\n".to_vec();

    // Fresh keypair.
    let sk = SigningKey::generate(&mut OsRng);
    let pk = sk.verifying_key();
    let sig = sk.sign(&cert_bytes).to_bytes();
    let sig_hex = hex_encode(&sig);

    write_cert_dir_with_one_obligation(
        &dir,
        "foo__ensures__0.smt2",
        &cert_bytes,
        Some(&sig_hex),
        None,
    );
    let pub_pem = write_pubkey_pem(&dir, &pk.to_bytes());

    let out = Command::new(bin())
        .args(["verify-all"])
        .arg(&dir)
        .arg("--pubkey")
        .arg(&pub_pem)
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "expected 0, stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("all checks passed"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_all_detects_signature_tamper() {
    let dir = tmp_dir("sig_tamper");
    let cert_bytes = b"payload v1".to_vec();

    let sk = SigningKey::generate(&mut OsRng);
    let pk = sk.verifying_key();
    // Sign DIFFERENT bytes than what we write to disk → sig
    // should not verify against the actual cert.
    let sig_on_other = sk.sign(b"payload v2").to_bytes();
    let sig_hex = hex_encode(&sig_on_other);

    write_cert_dir_with_one_obligation(
        &dir,
        "foo__ensures__0.smt2",
        &cert_bytes,
        Some(&sig_hex),
        None,
    );
    let pub_pem = write_pubkey_pem(&dir, &pk.to_bytes());

    let out = Command::new(bin())
        .args(["verify-all"])
        .arg(&dir)
        .arg("--pubkey")
        .arg(&pub_pem)
        .output()
        .expect("spawn");
    assert!(!out.status.success(), "should fail on sig mismatch");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_all_errors_on_missing_manifest() {
    let dir = tmp_dir("no_manifest");
    let out = Command::new(bin())
        .args(["verify-all"])
        .arg(&dir)
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("manifest.json"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_all_errors_on_malformed_manifest() {
    let dir = tmp_dir("bad_manifest");
    std::fs::write(dir.join("manifest.json"), "{ not json at all").unwrap();
    let out = Command::new(bin())
        .args(["verify-all"])
        .arg(&dir)
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_all_requires_directory_argument() {
    let out = Command::new(bin())
        .args(["verify-all"])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("<dir>") || stderr.contains("directory"),
        "stderr: {stderr}"
    );
}

#[test]
fn emit_certificates_writes_manifest_json() {
    // End-to-end through the binary: even with 0 obligations
    // (default build has no z3), emit-certificate should still
    // write a valid (empty-obligations) manifest.
    let dir = tmp_dir("e2e_manifest");
    let src = dir.join("prog.rs");
    std::fs::write(
        &src,
        "fn f(int x) { return x; } fn main(int _d) { return f(1); } main(0);\n",
    )
    .unwrap();
    let cert_dir = dir.join("certs");

    let out = Command::new(bin())
        .args(["-t", "--seed", "0", "--emit-certificate"])
        .arg(&cert_dir)
        .arg(&src)
        .output()
        .expect("spawn");
    assert!(out.status.success(), "expected success: {:?}", out);

    let manifest_path = cert_dir.join("manifest.json");
    assert!(manifest_path.exists(), "manifest.json must be written");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(manifest.contains("\"program\""), "manifest: {manifest}");
    assert!(manifest.contains("\"obligations\""), "manifest: {manifest}");

    // verify-all on the directory must succeed.
    let verify = Command::new(bin())
        .args(["verify-all"])
        .arg(&cert_dir)
        .output()
        .expect("spawn verify-all");
    assert!(
        verify.status.success(),
        "verify-all should succeed on a fresh emit: stdout={}, stderr={}",
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr),
    );

    let _ = std::fs::remove_dir_all(&dir);
}
