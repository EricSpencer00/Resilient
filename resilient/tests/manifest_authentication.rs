//! Strict verification authenticates the complete emitted enumeration.
#![cfg(feature = "z3")]

use ed25519_dalek::SigningKey;
use ed25519_dalek::ed25519::signature::Signer;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "res_manifest_auth_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let key = SigningKey::generate(&mut rand_core::OsRng);
        for (name, label, bytes) in [
            ("private.pem", "PRIVATE", key.to_bytes()),
            ("public.pem", "PUBLIC", key.verifying_key().to_bytes()),
        ] {
            std::fs::write(
                root.join(name),
                format!(
                    "-----BEGIN ED25519 {label} KEY-----\n{}\n-----END ED25519 {label} KEY-----\n",
                    hex(&bytes)
                ),
            )
            .unwrap();
        }
        let query = b"(assert false)\n(check-sat)\n";
        let obligations: Vec<_> = (0..2)
            .map(|idx| {
                let name = format!("proof-{idx}.smt2");
                std::fs::write(root.join(&name), query).unwrap();
                serde_json::json!({"fn":"f", "kind":"ensures", "idx":idx,
                "cert":name, "sha256":format!("{:x}", Sha256::digest(query)),
                "sig":hex(&key.sign(query).to_bytes())})
            })
            .collect();
        let manifest =
            serde_json::to_vec(&serde_json::json!({"program":"f.rz", "obligations":obligations}))
                .unwrap();
        std::fs::write(
            root.join("manifest.sig"),
            hex(&key.sign(&manifest).to_bytes()),
        )
        .unwrap();
        std::fs::write(root.join("manifest.json"), manifest).unwrap();
        Self(root)
    }

    fn edit_manifest(&self, edit: impl FnOnce(&mut serde_json::Value)) {
        let path = self.0.join("manifest.json");
        let mut value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        edit(&mut value);
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }

    fn check(&self, directory: &Path, strict: bool, accepted: bool) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
        command
            .arg("verify-all")
            .arg(directory)
            .arg("--pubkey")
            .arg(self.0.join("public.pem"))
            .arg("--z3")
            .env("Z3_BINARY", "z3");
        if strict {
            command.arg("--require-manifest-signature");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.success(), accepted, "{output:?}");
        if !accepted {
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("manifest signature"),
                "{output:?}"
            );
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn strict_replay_accepts_the_complete_signed_manifest() {
    let fixture = Fixture::new();
    fixture.check(&fixture.0, true, true);
}

#[test]
fn strict_replay_rejects_removed_obligations() {
    let fixture = Fixture::new();
    fixture.edit_manifest(|m| {
        m["obligations"].as_array_mut().unwrap().pop();
    });
    fixture.check(&fixture.0, true, false);
}

#[test]
fn strict_replay_rejects_reordered_obligations() {
    let fixture = Fixture::new();
    fixture.edit_manifest(|m| m["obligations"].as_array_mut().unwrap().reverse());
    fixture.check(&fixture.0, true, false);
}

#[test]
fn strict_replay_rejects_changed_program_identity() {
    let fixture = Fixture::new();
    fixture.edit_manifest(|m| m["program"] = serde_json::json!("different.rz"));
    fixture.check(&fixture.0, true, false);
}

#[test]
fn strict_replay_rejects_missing_manifest_signature() {
    let fixture = Fixture::new();
    std::fs::remove_file(fixture.0.join("manifest.sig")).unwrap();
    fixture.check(&fixture.0, true, false);
}

#[test]
fn strict_replay_rejects_wrong_manifest_key() {
    let fixture = Fixture::new();
    let key = SigningKey::generate(&mut rand_core::OsRng);
    std::fs::write(
        fixture.0.join("public.pem"),
        format!(
            "-----BEGIN ED25519 PUBLIC KEY-----\n{}\n-----END ED25519 PUBLIC KEY-----\n",
            hex(&key.verifying_key().to_bytes())
        ),
    )
    .unwrap();
    fixture.check(&fixture.0, true, false);
}

#[test]
fn legacy_replay_does_not_require_a_manifest_signature() {
    let fixture = Fixture::new();
    std::fs::remove_file(fixture.0.join("manifest.sig")).unwrap();
    fixture.check(&fixture.0, false, true);
}

#[test]
fn compiler_signs_the_manifest_for_strict_replay() {
    let fixture = Fixture::new();
    let source = fixture.0.join("program.rz");
    std::fs::write(
        &source,
        "let n = 0; while n < 3 invariant n >= 0 { n = n + 1; }",
    )
    .unwrap();
    let certs = fixture.0.join("generated");
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "-t", "--emit-certificate"])
        .arg(&certs)
        .arg("--sign-cert")
        .arg(fixture.0.join("private.pem"))
        .arg(source)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        certs.join("manifest.sig").is_file(),
        "the emitted enumeration must be signed"
    );
    fixture.check(&certs, true, true);
}
