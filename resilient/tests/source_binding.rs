//! Authenticated frozen translation inputs, independently checked through the CLI.
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

struct Fixture {
    root: PathBuf,
    key: SigningKey,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "source_binding_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("inputs")).unwrap();
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
        std::fs::write(
            root.join("inputs/program.rz"),
            "use \"dep.rz\"; let n = 0; while n < 3 invariant n >= 0 { n = n + 1; }",
        )
        .unwrap();
        std::fs::write(
            root.join("inputs/dep.rz"),
            "pub fn identity(int x) -> int { return x; }",
        )
        .unwrap();
        let fixture = Self { root, key };
        fixture.emit("certs", true);
        fixture
    }

    fn emit(&self, destination: &str, no_cache: bool) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
        command
            .arg("-t")
            .arg("--emit-certificate")
            .arg(self.root.join(destination))
            .arg("--sign-cert")
            .arg(self.root.join("private.pem"))
            .arg(self.root.join("inputs/program.rz"));
        if no_cache {
            command.arg("--no-cache");
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    fn manifest(&self) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(self.root.join("certs/manifest.json")).unwrap())
            .unwrap()
    }

    fn resign(&self, edit: impl FnOnce(&mut serde_json::Value)) {
        let mut value = self.manifest();
        edit(&mut value);
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(
            self.root.join("certs/manifest.sig"),
            hex(&self.key.sign(&bytes).to_bytes()),
        )
        .unwrap();
        std::fs::write(self.root.join("certs/manifest.json"), bytes).unwrap();
    }

    fn check(&self, root: Option<&Path>, expected: bool) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
        command
            .arg("verify-all")
            .arg(self.root.join("certs"))
            .arg("--pubkey")
            .arg(self.root.join("public.pem"))
            .arg("--z3")
            .env("Z3_BINARY", "z3");
        if let Some(root) = root {
            command.arg("--source-root").arg(root);
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.success(), expected, "{output:?}");
        if !expected {
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains("source binding") || error.contains("manifest signature"),
                "{output:?}"
            );
        }
    }

    fn strict(&self, expected: bool) {
        self.check(Some(&self.root.join("inputs")), expected);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn signed_inventory_binds_all_parsed_inputs() {
    let f = Fixture::new();
    let value = f.manifest();
    let binding = &value["source_binding"];
    assert_eq!(binding["version"], 1);
    assert_eq!(binding["entry"], "program.rz");
    let files = binding["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    for file in files {
        let bytes =
            std::fs::read(f.root.join("inputs").join(file["path"].as_str().unwrap())).unwrap();
        assert_eq!(file["sha256"], format!("{:x}", Sha256::digest(&bytes)));
        assert_eq!(file["bytes"], bytes.len() as u64);
    }
    f.strict(true);
}

#[test]
fn identical_relocated_sources_verify() {
    let f = Fixture::new();
    let relocated = f.root.join("relocated");
    std::fs::create_dir(&relocated).unwrap();
    for file in ["program.rz", "dep.rz"] {
        std::fs::copy(f.root.join("inputs").join(file), relocated.join(file)).unwrap();
    }
    f.check(Some(&relocated), true);
}

#[test]
fn changed_entry_is_rejected() {
    let f = Fixture::new();
    std::fs::write(f.root.join("inputs/program.rz"), "let changed = 1;").unwrap();
    f.strict(false);
}

#[test]
fn changed_import_is_rejected() {
    let f = Fixture::new();
    std::fs::write(
        f.root.join("inputs/dep.rz"),
        "pub fn identity(int x) -> int { return 0; }",
    )
    .unwrap();
    f.strict(false);
}

#[test]
fn missing_inventory_is_rejected() {
    let f = Fixture::new();
    f.resign(|m| {
        m.as_object_mut().unwrap().remove("source_binding");
    });
    f.strict(false);
}

#[test]
fn unsigned_inventory_is_rejected() {
    let f = Fixture::new();
    std::fs::remove_file(f.root.join("certs/manifest.sig")).unwrap();
    f.strict(false);
}

#[test]
fn malformed_inventory_is_rejected() {
    let f = Fixture::new();
    f.resign(|m| {
        m["source_binding"]["files"][0]["sha256"] = "bad".into();
    });
    f.strict(false);
}

#[test]
fn duplicate_inventory_file_is_rejected() {
    let f = Fixture::new();
    f.resign(|m| {
        let files = m["source_binding"]["files"].as_array_mut().unwrap();
        files.push(files[0].clone());
    });
    f.strict(false);
}

#[test]
fn missing_import_edge_is_rejected() {
    let f = Fixture::new();
    f.resign(|m| {
        m["source_binding"]["imports"] = serde_json::json!([]);
    });
    f.strict(false);
}

#[test]
fn unreachable_inventory_file_is_rejected() {
    let f = Fixture::new();
    std::fs::write(f.root.join("inputs/unreachable.rz"), "let unused = 1;").unwrap();
    f.resign(|m| { m["source_binding"]["files"].as_array_mut().unwrap().push(serde_json::json!({"path":"unreachable.rz", "sha256":format!("{:x}",Sha256::digest(b"let unused = 1;")),"bytes":15})); });
    f.strict(false);
}

#[test]
fn traversal_inventory_path_is_rejected() {
    let f = Fixture::new();
    f.resign(|m| {
        m["source_binding"]["files"][0]["path"] = "../outside.rz".into();
    });
    f.strict(false);
}

#[cfg(unix)]
#[test]
fn symlink_import_outside_root_is_rejected_before_read() {
    let f = Fixture::new();
    let dep = f.root.join("inputs/dep.rz");
    std::fs::rename(&dep, f.root.join("outside.rz")).unwrap();
    std::os::unix::fs::symlink(f.root.join("outside.rz"), dep).unwrap();
    f.strict(false);
}

#[test]
fn cache_and_no_cache_emit_identical_binding() {
    let f = Fixture::new();
    f.emit("cached", false);
    f.emit("cache_hit", false);
    let binding = f.manifest()["source_binding"].clone();
    assert!(!binding.is_null());
    for directory in ["cached", "cache_hit"] {
        let m: serde_json::Value = serde_json::from_slice(
            &std::fs::read(f.root.join(directory).join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(m["source_binding"], binding);
    }
}

#[test]
fn legacy_replay_remains_compatible_without_binding() {
    let f = Fixture::new();
    f.resign(|m| {
        m.as_object_mut().unwrap().remove("source_binding");
    });
    std::fs::remove_file(f.root.join("certs/manifest.sig")).unwrap();
    f.check(None, true);
}
