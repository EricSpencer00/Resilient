//! Requested solver replay must never succeed without checking an obligation.
#![cfg(feature = "z3")]

use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Bundle(PathBuf);

impl Bundle {
    fn new(empty: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "res_replay_required_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let query = b"(assert false)\n(check-sat)\n";
        std::fs::write(root.join("proof.smt2"), query).unwrap();
        let obligations = if empty {
            serde_json::json!([])
        } else {
            serde_json::json!([{"fn": "f", "kind": "ensures", "idx": 0,
                "cert": "proof.smt2", "sha256": format!("{:x}", Sha256::digest(query))}])
        };
        std::fs::write(
            root.join("manifest.json"),
            serde_json::json!({"program": "f.rz", "obligations": obligations}).to_string(),
        )
        .unwrap();
        Self(root)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
        command.arg("verify-all").arg(&self.0);
        command
    }
}

impl Drop for Bundle {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn rejected(output: Output) {
    assert!(!output.status.success(), "{output:?}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("all checks passed"),
        "{output:?}"
    );
}

#[test]
fn requested_replay_rejects_missing_absolute_solver() {
    let bundle = Bundle::new(false);
    rejected(
        bundle
            .command()
            .arg("--z3")
            .env("Z3_BINARY", bundle.0.join("does-not-exist"))
            .output()
            .unwrap(),
    );
}

#[test]
fn requested_replay_rejects_solver_absent_from_path() {
    let bundle = Bundle::new(false);
    rejected(
        bundle
            .command()
            .arg("--z3")
            .env_remove("Z3_BINARY")
            .env("PATH", &bundle.0)
            .output()
            .unwrap(),
    );
}

#[test]
fn requested_replay_rejects_empty_obligation_set() {
    let bundle = Bundle::new(true);
    rejected(
        bundle
            .command()
            .arg("--z3")
            .env("Z3_BINARY", env!("CARGO_BIN_EXE_rz"))
            .output()
            .unwrap(),
    );
}

#[test]
fn hash_only_verification_does_not_require_solver() {
    let bundle = Bundle::new(false);
    let output = bundle
        .command()
        .env("Z3_BINARY", bundle.0.join("does-not-exist"))
        .env("PATH", &bundle.0)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("all checks passed"));
}
