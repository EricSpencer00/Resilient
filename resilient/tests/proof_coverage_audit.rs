//! Requested implementation coverage is separate from signed replay of a subset.
#![cfg(feature = "z3")]

use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn canonical(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

struct Fixture {
    root: PathBuf,
    key: SigningKey,
}
impl Fixture {
    fn new(source: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "proof_coverage_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("inputs")).unwrap();
        std::fs::write(root.join("inputs/program.rz"), source).unwrap();
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
        let result = Command::new(env!("CARGO_BIN_EXE_rz"))
            .args(["--no-cache", "-t", "--emit-certificate"])
            .arg(root.join("certs"))
            .arg("--sign-cert")
            .arg(root.join("private.pem"))
            .arg(root.join("inputs/program.rz"))
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        Self { root, key }
    }
    fn manifest(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.root.join("certs/manifest.json")).unwrap())
            .unwrap()
    }
    fn save_manifest(&self, manifest: &Value) {
        let bytes = canonical(manifest);
        std::fs::write(
            self.root.join("certs/manifest.sig"),
            hex(&self.key.sign(&bytes).to_bytes()),
        )
        .unwrap();
        std::fs::write(self.root.join("certs/manifest.json"), bytes).unwrap();
    }
    fn replay(&self) {
        let result = Command::new(env!("CARGO_BIN_EXE_rz"))
            .arg("verify-all")
            .arg(self.root.join("certs"))
            .arg("--source-root")
            .arg(self.root.join("inputs"))
            .arg("--pubkey")
            .arg(self.root.join("public.pem"))
            .arg("--z3")
            .env("Z3_BINARY", "z3")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "mutation must remain authenticated and replayable: {result:?}"
        );
    }
    fn install_queries(&self, count: usize) {
        let mut manifest = self.manifest();
        let mut entries = Vec::new();
        for idx in 0..count {
            let query = query(idx);
            let name = format!("reviewed_{idx}.smt2");
            std::fs::write(self.root.join("certs").join(&name), query).unwrap();
            entries.push(json!({"fn":"covered:ensures:0","kind":"ensures_implementation","idx":idx,"cert":name,"sha256":hash(query.as_bytes()),"sig":hex(&self.key.sign(query.as_bytes()).to_bytes())}));
        }
        manifest["obligations"] = json!(entries);
        self.save_manifest(&manifest);
    }
    fn policy(&self, count: usize) -> Value {
        let manifest = self.manifest();
        let sites: Vec<_> = (0..count).map(|idx| json!({"source":"program.rz","declaration":"program::covered","clause":0,"path":if idx==0 {"then"}else{"else"},"status":"required","obligation":{"fn":"covered:ensures:0","kind":"ensures_implementation","idx":idx},"commands":commands(idx)})).collect();
        json!({"schema":"resilient-expected-implementation-coverage","version":1,"profile":"reviewed-integer-equality-v1","compiler":{"sha256":hash(&std::fs::read(env!("CARGO_BIN_EXE_rz")).unwrap()),"build_profile":"debug"},"source_graph_sha256":hash(&canonical(&manifest["source_binding"])),"sites":sites})
    }
    fn audit(&self, policy: &Value, expected: &str) {
        let bytes = canonical(policy);
        std::fs::write(self.root.join("policy.json"), &bytes).unwrap();
        let result = self.invoke(&hash(&bytes), "reviewed-integer-equality-v1");
        let report: Value = serde_json::from_slice(&result.stdout)
            .unwrap_or_else(|_| panic!("auditor must produce structured result: {result:?}"));
        assert_eq!(report["code"], expected, "{report}");
        assert_eq!(result.status.success(), expected == "covered", "{report}");
        if expected != "covered" {
            assert_eq!(
                report["stage"], "coverage",
                "authenticated mutation must reach coverage gate: {report}"
            );
        }
    }
    fn invoke(&self, policy_hash: &str, profile: &str) -> Output {
        Command::new("python3")
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../agent-scripts/audit-proof-coverage.py"),
            )
            .arg("--rz")
            .arg(env!("CARGO_BIN_EXE_rz"))
            .arg("--cert-dir")
            .arg(self.root.join("certs"))
            .arg("--source-root")
            .arg(self.root.join("inputs"))
            .arg("--pubkey")
            .arg(self.root.join("public.pem"))
            .arg("--policy")
            .arg(self.root.join("policy.json"))
            .arg("--policy-sha256")
            .arg(policy_hash)
            .arg("--profile")
            .arg(profile)
            .arg("--timeout-seconds")
            .arg("20")
            .env("Z3_BINARY", "z3")
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(root) = std::env::var("COVERAGE_RECORD_DIR") {
            let destination = PathBuf::from(root).join(self.root.file_name().unwrap());
            std::fs::create_dir_all(&destination).unwrap();
            for directory in ["inputs", "certs"] {
                std::fs::create_dir(destination.join(directory)).unwrap();
                for entry in std::fs::read_dir(self.root.join(directory)).unwrap() {
                    let entry = entry.unwrap();
                    std::fs::copy(
                        entry.path(),
                        destination.join(directory).join(entry.file_name()),
                    )
                    .unwrap();
                }
            }
            for name in ["public.pem", "policy.json"] {
                if self.root.join(name).exists() {
                    std::fs::copy(self.root.join(name), destination.join(name)).unwrap();
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// Independent equality-only source checkpoints. Each path returns the original x;
// antecedents partition x at zero and the negated postcondition is x != x.
fn commands(idx: usize) -> Value {
    let guard = if idx == 0 {
        json!([">=", "x", "0"])
    } else {
        json!(["not", [">=", "x", "0"]])
    };
    json!([
        ["set-logic", "AUFLIA"],
        ["declare-const", "x", "Int"],
        ["assert", guard],
        ["assert", ["not", ["=", "x", "x"]]],
        ["check-sat"]
    ])
}
fn query(idx: usize) -> &'static str {
    if idx == 0 {
        "(set-logic AUFLIA)\n(declare-const x Int)\n(assert (>= x 0))\n(assert (not (= x x)))\n(check-sat)\n"
    } else {
        "(set-logic AUFLIA)\n(declare-const x Int)\n(assert (not (>= x 0)))\n(assert (not (= x x)))\n(check-sat)\n"
    }
}
fn branch() -> Fixture {
    let f = Fixture::new(
        "fn covered(int x) -> int ensures result == x { if x >= 0 { return x; } else { return x; } }",
    );
    f.install_queries(2);
    f
}

#[test]
fn legacy_replay_accepts_the_mixed_subset_control() {
    let f = Fixture::new(
        "fn covered(int x) -> int ensures result == x { return x; } fn uncovered(int x) -> int ensures result == x { let y = x + 1; return y; }",
    );
    f.replay();
    assert!(
        !f.manifest()["obligations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["fn"].as_str().unwrap().starts_with("uncovered:"))
    );
}
#[test]
fn covered_independent_branch_checkpoints_pass() {
    let f = branch();
    f.replay();
    f.audit(&f.policy(2), "covered");
}
#[test]
fn unsupported_wrong_let_remains_explicitly_unresolved() {
    let f = Fixture::new(
        "fn covered(int x) -> int ensures result == x { return x; } fn uncovered(int x) -> int ensures result == x { let y = x + 1; return y; }",
    );
    f.replay();
    let mut p = f.policy(1);
    p["sites"][0]["status"] = json!("unsupported");
    p["sites"][0]["declaration"] = json!("program::uncovered");
    p["sites"][0]["path"] = json!("return");
    f.audit(&p, "unresolved_site");
}
#[test]
fn missing_path_is_rejected_after_valid_authentication() {
    let f = branch();
    let p = f.policy(2);
    let mut m = f.manifest();
    m["obligations"].as_array_mut().unwrap().pop();
    f.save_manifest(&m);
    f.replay();
    f.audit(&p, "missing_obligation");
}
#[test]
fn duplicate_path_is_rejected_after_valid_authentication() {
    let f = branch();
    let p = f.policy(2);
    let mut m = f.manifest();
    let entry = m["obligations"][0].clone();
    m["obligations"].as_array_mut().unwrap().push(entry);
    f.save_manifest(&m);
    f.replay();
    f.audit(&p, "duplicate_obligation");
}
#[test]
fn extra_path_is_rejected_after_valid_authentication() {
    let f = branch();
    let p = f.policy(1);
    f.replay();
    f.audit(&p, "extra_obligation");
}
#[test]
fn clause_only_does_not_satisfy_implementation_site() {
    let f = branch();
    let p = f.policy(2);
    let mut m = f.manifest();
    m["obligations"][0]["kind"] = json!("ensures");
    f.save_manifest(&m);
    f.replay();
    f.audit(&p, "wrong_basis");
}
#[test]
fn valid_unsat_query_with_wrong_path_guard_is_rejected() {
    let f = branch();
    let p = f.policy(2);
    let mut m = f.manifest();
    let mut first = m["obligations"][0].clone();
    first["idx"] = json!(1);
    m["obligations"][1] = first;
    f.save_manifest(&m);
    f.replay();
    f.audit(&p, "checkpoint_mismatch");
}
#[test]
fn duplicate_policy_site_is_rejected() {
    let f = branch();
    let mut p = f.policy(2);
    let site = p["sites"][0].clone();
    p["sites"].as_array_mut().unwrap().push(site);
    f.audit(&p, "duplicate_site");
}
#[test]
fn infeasible_site_is_not_misreported_as_covered() {
    let f = branch();
    let mut p = f.policy(2);
    p["sites"][0]["status"] = json!("infeasible");
    f.audit(&p, "unresolved_site");
}
#[test]
fn explicit_unselected_site_is_distinct_from_unresolved() {
    let f = branch();
    let mut p = f.policy(2);
    p["sites"].as_array_mut().unwrap().push(json!({"source":"program.rz","declaration":"program::not_requested","clause":0,"path":"return","status":"unselected"}));
    f.audit(&p, "covered");
}
#[test]
fn policy_hash_binding_is_required() {
    let f = branch();
    let p = f.policy(2);
    let bytes = canonical(&p);
    std::fs::write(f.root.join("policy.json"), bytes).unwrap();
    let result = f.invoke(&"0".repeat(64), "reviewed-integer-equality-v1");
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["code"], "policy_hash_mismatch");
    assert!(!result.status.success());
}
#[test]
fn compiler_binding_is_required() {
    let f = branch();
    let mut p = f.policy(2);
    p["compiler"]["sha256"] = json!("0".repeat(64));
    let bytes = canonical(&p);
    std::fs::write(f.root.join("policy.json"), &bytes).unwrap();
    let result = f.invoke(&hash(&bytes), "reviewed-integer-equality-v1");
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["code"], "compiler_hash_mismatch");
    assert!(!result.status.success());
}
#[test]
fn source_graph_binding_is_required() {
    let f = branch();
    let mut p = f.policy(2);
    p["source_graph_sha256"] = json!("0".repeat(64));
    let bytes = canonical(&p);
    std::fs::write(f.root.join("policy.json"), &bytes).unwrap();
    let result = f.invoke(&hash(&bytes), "reviewed-integer-equality-v1");
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["code"], "source_graph_mismatch");
    assert!(!result.status.success());
}
#[test]
fn profile_binding_is_required() {
    let f = branch();
    let p = f.policy(2);
    let bytes = canonical(&p);
    std::fs::write(f.root.join("policy.json"), &bytes).unwrap();
    let result = f.invoke(&hash(&bytes), "different-profile");
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["code"], "profile_mismatch");
    assert!(!result.status.success());
}
#[test]
fn auditor_performs_source_authentication_itself() {
    let f = branch();
    let p = f.policy(2);
    std::fs::write(
        f.root.join("inputs/program.rz"),
        "fn changed() -> int { return 0; }",
    )
    .unwrap();
    let bytes = canonical(&p);
    std::fs::write(f.root.join("policy.json"), &bytes).unwrap();
    let result = f.invoke(&hash(&bytes), "reviewed-integer-equality-v1");
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["stage"], "authentication");
    assert_eq!(report["code"], "authentication_failed");
    assert!(!result.status.success());
}
#[test]
fn non_result_clause_is_not_silently_implementation_covered() {
    let f = branch();
    let mut p = f.policy(2);
    p["sites"][0]["status"] = json!("unresolved");
    p["sites"][0]["reason"] =
        json!("declared ensures without result requires independent final-state model");
    f.audit(&p, "unresolved_site");
}
#[test]
fn duplicate_guard_assertion_is_not_normalized_away() {
    let f = branch();
    let p = f.policy(2);
    let mut m = f.manifest();
    let query = "(set-logic AUFLIA)\n(declare-const x Int)\n(assert (>= x 0))\n(assert (>= x 0))\n(assert (not (= x x)))\n(check-sat)\n";
    let name = m["obligations"][0]["cert"].as_str().unwrap();
    std::fs::write(f.root.join("certs").join(name), query).unwrap();
    m["obligations"][0]["sha256"] = json!(hash(query.as_bytes()));
    m["obligations"][0]["sig"] = json!(hex(&f.key.sign(query.as_bytes()).to_bytes()));
    f.save_manifest(&m);
    f.replay();
    f.audit(&p, "checkpoint_mismatch");
}
