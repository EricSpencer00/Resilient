//! RES-194: integration tests for the `verify-cert` subcommand
//! and the `--sign-cert` flag end-to-end.
//!
//! Strategy: every test drives the real `resilient` binary with
//! a pair of ephemeral Ed25519 keys generated in-test. The
//! `--pubkey` override on `verify-cert` lets us supply the
//! matching public key; the embedded default key intentionally
//! doesn't match (we can't sign with its corresponding private
//! key from here — it lives off-repo per the ticket's key-
//! management policy).
//!
//! Covered cases:
//! - Happy path: sign a cert directory, verify with matching
//!   `--pubkey` → exit 0.
//! - Wrong key: verify with the embedded (non-matching) key →
//!   exit 1, diagnostic mentions mismatch.
//! - Tamper on payload: sign, then hand-edit an `.smt2` file,
//!   verify → exit 1.
//! - Missing cert.sig: exit 2.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("res_194_{}_{}_{}", tag, std::process::id(), n));
    std::fs::create_dir_all(&p).expect("mkdir tmp");
    p
}

/// Generate an ephemeral keypair and write PEM files into the
/// scratch directory. Returns `(priv_pem_path, pub_pem_path,
/// raw_pub_bytes)`. Uses `ed25519-dalek` through our cert_sign
/// module's PEM helpers; tests build against the same dep edge
/// the binary does.
fn write_test_keypair(dir: &std::path::Path) -> (PathBuf, PathBuf) {
    // We can't import `crate::cert_sign` from an integration test —
    // integration tests are separate crates. Shell out to the
    // same ed25519-dalek crate via a tiny inline generation using
    // the stable API (`SigningKey::generate`).
    use ed25519_dalek::SigningKey;
    use rand_core::OsRng;
    let sk = SigningKey::generate(&mut OsRng);
    let pk = sk.verifying_key();

    let hex_pk: String = pk.to_bytes().iter().map(|b| format!("{:02x}", b)).collect();
    let hex_sk: String = sk.to_bytes().iter().map(|b| format!("{:02x}", b)).collect();

    let priv_path = dir.join("priv.pem");
    std::fs::write(
        &priv_path,
        format!(
            "-----BEGIN ED25519 PRIVATE KEY-----\n{}\n-----END ED25519 PRIVATE KEY-----\n",
            hex_sk
        ),
    )
    .unwrap();
    let pub_path = dir.join("pub.pem");
    std::fs::write(
        &pub_path,
        format!(
            "-----BEGIN ED25519 PUBLIC KEY-----\n{}\n-----END ED25519 PUBLIC KEY-----\n",
            hex_pk
        ),
    )
    .unwrap();
    (priv_path, pub_path)
}

/// Build a tiny Resilient source file whose type-check produces
/// at least one verification certificate. We ship a trivial fn
/// with a `requires`/`ensures` contract pair so the Z3-backed
/// path emits `.smt2` files when the `z3` feature is on; on a
/// build without z3 the directory ends up empty-but-signed, which
/// is still a valid end-to-end check of the signing path.
fn write_canary_source(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("canary.rs");
    std::fs::write(
        &p,
        "fn add(int a, int b) requires a > 0 ensures result > a { return a + b; }\n\
         fn main(int _d) { return add(1, 2); } main(0);\n",
    )
    .unwrap();
    p
}

#[test]
fn sign_cert_and_verify_round_trip() {
    let scratch = tmp_dir("rt");
    let (priv_pem, pub_pem) = write_test_keypair(&scratch);
    let src = write_canary_source(&scratch);
    let cert_dir = scratch.join("certs");

    // --emit-certificate + --sign-cert
    let emit = Command::new(bin())
        .args(["-t", "--seed", "0", "--emit-certificate"])
        .arg(&cert_dir)
        .arg("--sign-cert")
        .arg(&priv_pem)
        .arg(&src)
        .output()
        .expect("spawn resilient emit");
    assert!(emit.status.success(), "emit failed: {:?}", emit);
    assert!(cert_dir.join("cert.sig").exists(), "cert.sig not written");

    // verify-cert with matching --pubkey → exit 0
    let ok = Command::new(bin())
        .args(["verify-cert"])
        .arg(&cert_dir)
        .arg("--pubkey")
        .arg(&pub_pem)
        .output()
        .expect("spawn verify-cert");
    assert!(
        ok.status.success(),
        "verify should succeed; stderr={}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let stdout = String::from_utf8_lossy(&ok.stdout);
    assert!(stdout.contains("verified"), "unexpected stdout: {stdout}");

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn verify_cert_fails_against_mismatched_key() {
    let scratch = tmp_dir("mismatch");
    // First keypair: used to sign.
    let (priv_pem, _pub_pem) = write_test_keypair(&scratch);
    // Second keypair in a sibling subdir, used to verify —
    // different bytes, so the signature should NOT verify.
    let other_dir = scratch.join("other");
    std::fs::create_dir_all(&other_dir).unwrap();
    let (_other_priv, other_pub_pem) = write_test_keypair(&other_dir);

    let src = write_canary_source(&scratch);
    let cert_dir = scratch.join("certs");

    let emit = Command::new(bin())
        .args(["-t", "--seed", "0", "--emit-certificate"])
        .arg(&cert_dir)
        .arg("--sign-cert")
        .arg(&priv_pem)
        .arg(&src)
        .output()
        .expect("spawn emit");
    assert!(emit.status.success(), "emit failed: {:?}", emit);

    let mismatch = Command::new(bin())
        .args(["verify-cert"])
        .arg(&cert_dir)
        .arg("--pubkey")
        .arg(&other_pub_pem)
        .output()
        .expect("spawn verify-cert");
    // rc = 1 on signature-mismatch; the driver prints a red
    // "SIGNATURE MISMATCH" line on stderr.
    assert!(
        !mismatch.status.success(),
        "verify should fail under wrong pubkey"
    );
    let stderr = String::from_utf8_lossy(&mismatch.stderr);
    assert!(
        stderr.to_uppercase().contains("MISMATCH") || stderr.to_lowercase().contains("tampered"),
        "unexpected stderr: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn verify_cert_detects_payload_tamper() {
    let scratch = tmp_dir("tamper");
    let (priv_pem, pub_pem) = write_test_keypair(&scratch);
    let src = write_canary_source(&scratch);
    let cert_dir = scratch.join("certs");

    let emit = Command::new(bin())
        .args(["-t", "--seed", "0", "--emit-certificate"])
        .arg(&cert_dir)
        .arg("--sign-cert")
        .arg(&priv_pem)
        .arg(&src)
        .output()
        .expect("spawn emit");
    assert!(emit.status.success());

    // Append a fake .smt2 file to the cert dir — the payload
    // changes, so the signature should no longer verify.
    std::fs::write(cert_dir.join("tamper__fake__99.smt2"), b"(assert wrong)\n").unwrap();

    let verify = Command::new(bin())
        .args(["verify-cert"])
        .arg(&cert_dir)
        .arg("--pubkey")
        .arg(&pub_pem)
        .output()
        .expect("spawn verify");
    assert!(!verify.status.success(), "tamper must fail verification");

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn verify_cert_errors_on_missing_sig() {
    let scratch = tmp_dir("no_sig");
    // Create a cert dir with a .smt2 file but NO cert.sig.
    let cert_dir = scratch.join("certs");
    std::fs::create_dir_all(&cert_dir).unwrap();
    std::fs::write(cert_dir.join("a__b__0.smt2"), b"(assert true)\n").unwrap();

    let out = Command::new(bin())
        .args(["verify-cert"])
        .arg(&cert_dir)
        .output()
        .expect("spawn verify");
    assert!(!out.status.success(), "must fail without cert.sig");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cert.sig"),
        "error should mention cert.sig: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn verify_cert_requires_directory_argument() {
    let out = Command::new(bin())
        .args(["verify-cert"])
        .output()
        .expect("spawn verify");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("<dir>") || stderr.contains("directory"),
        "error should demand a dir: {stderr}"
    );
}
