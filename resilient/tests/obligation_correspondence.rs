//! Exact source checkpoints remain necessary when all mutated sessions are UNSAT.
#![cfg(all(feature = "z3", unix))]
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn render(node: &Value) -> String {
    if let Some(items) = node.as_array() {
        format!(
            "({})",
            items.iter().map(render).collect::<Vec<_>>().join(" ")
        )
    } else {
        node.as_str().unwrap().into()
    }
}
struct Fixture {
    root: PathBuf,
    key: SigningKey,
    descriptor: Value,
}
impl Fixture {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let data =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proof_correspondence");
        let descriptor: Value = serde_json::from_slice(
            &std::fs::read(data.join(format!("legacy_{name}.json"))).unwrap(),
        )
        .unwrap();
        let root = std::env::temp_dir().join(format!(
            "structural_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("inputs")).unwrap();
        std::fs::copy(
            data.join(format!("{name}.rz")),
            root.join("inputs/program.rz"),
        )
        .unwrap();
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
        let output = Command::new(env!("CARGO_BIN_EXE_rz"))
            .args(["--no-cache", "-t", "--emit-certificate"])
            .arg(root.join("certs"))
            .arg("--sign-cert")
            .arg(root.join("private.pem"))
            .arg(root.join("inputs/program.rz"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "source frontend failure is not a structural result: {output:?}"
        );
        Self {
            root,
            key,
            descriptor,
        }
    }
    fn manifest(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.root.join("certs/manifest.json")).unwrap())
            .unwrap()
    }
    fn save(&self, manifest: &Value) {
        let raw = serde_json::to_vec(manifest).unwrap();
        std::fs::write(
            self.root.join("certs/manifest.sig"),
            hex(&self.key.sign(&raw).to_bytes()),
        )
        .unwrap();
        std::fs::write(self.root.join("certs/manifest.json"), raw).unwrap();
    }
    fn commands(&self, idx: usize) -> Value {
        self.descriptor["paths"][idx]["commands"].clone()
    }
    fn replace(&self, idx: usize, commands: &Value) {
        let raw = commands
            .as_array()
            .unwrap()
            .iter()
            .map(render)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let mut manifest = self.manifest();
        let entry = manifest["obligations"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["kind"] == "ensures_implementation" && entry["idx"] == idx)
            .unwrap();
        std::fs::write(
            self.root
                .join("certs")
                .join(entry["cert"].as_str().unwrap()),
            &raw,
        )
        .unwrap();
        entry["sha256"] = json!(hash(raw.as_bytes()));
        entry["sig"] = json!(hex(&self.key.sign(raw.as_bytes()).to_bytes()));
        self.save(&manifest);
    }
    fn check(&self, expected: &str) {
        let replay = Command::new(env!("CARGO_BIN_EXE_rz"))
            .arg("verify-all")
            .arg(self.root.join("certs"))
            .arg("--source-root")
            .arg(self.root.join("inputs"))
            .arg("--pubkey")
            .arg(self.root.join("public.pem"))
            .arg("--z3")
            .output()
            .unwrap();
        assert!(
            replay.status.success(),
            "control must remain signed-valid and stock-Z3 replayable: {replay:?}"
        );
        self.invoke(expected, true);
    }
    fn invoke(&self, expected: &str, authenticated: bool) {
        let descriptor_raw = serde_json::to_vec(&self.descriptor).unwrap();
        std::fs::write(self.root.join("descriptor.json"), &descriptor_raw).unwrap();
        let solver = Command::new("which").arg("z3").output().unwrap();
        assert!(solver.status.success());
        let solver = String::from_utf8(solver.stdout).unwrap();
        let solver = solver.trim();
        let compiler_sha = hash(&std::fs::read(env!("CARGO_BIN_EXE_rz")).unwrap());
        let solver_sha = hash(&std::fs::read(solver).unwrap());
        let output = Command::new("python3")
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../agent-scripts/proof-correspondence-checker.py"),
            )
            .arg("--rz")
            .arg(env!("CARGO_BIN_EXE_rz"))
            .arg("--compiler-sha256")
            .arg(&compiler_sha)
            .arg("--solver")
            .arg(solver)
            .arg("--solver-sha256")
            .arg(&solver_sha)
            .arg("--cert-dir")
            .arg(self.root.join("certs"))
            .arg("--source-root")
            .arg(self.root.join("inputs"))
            .arg("--pubkey")
            .arg(self.root.join("public.pem"))
            .arg("--descriptor")
            .arg(self.root.join("descriptor.json"))
            .arg("--descriptor-sha256")
            .arg(hash(&descriptor_raw))
            .arg("--profile")
            .arg("legacy-lia-identity-v1")
            .arg("--build-profile")
            .arg("debug")
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| json!({"unavailable":String::from_utf8_lossy(&output.stderr),"raw":String::from_utf8_lossy(&output.stdout)}));
        let manifest = self.manifest();
        let queries: Vec<_> = manifest["obligations"].as_array().unwrap().iter().map(|entry| json!({"entry":entry,"text":std::fs::read_to_string(self.root.join("certs").join(entry["cert"].as_str().unwrap())).unwrap()})).collect();
        let record = json!({"case":std::thread::current().name(),"claim":"exact checkpoint-policy comparison; equivalent-expression controls are not numerical counterexamples","descriptor":self.descriptor,"descriptor_sha256":hash(&descriptor_raw),"source":std::fs::read_to_string(self.root.join("inputs/program.rz")).unwrap(),"manifest":manifest,"manifest_signature":std::fs::read_to_string(self.root.join("certs/manifest.sig")).unwrap(),"public_key":std::fs::read_to_string(self.root.join("public.pem")).unwrap(),"queries":queries,"compiler_sha256":compiler_sha,"solver_sha256":solver_sha,"expected":expected,"preauthentication":authenticated,"report":report});
        let raw = serde_json::to_vec(&record).unwrap();
        assert!(raw.len() < 256 * 1024);
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(b"CORRESPONDENCE_STRUCTURE_CAPTURE ")
            .unwrap();
        stdout.write_all(&raw).unwrap();
        stdout.write_all(b"\n").unwrap();
        drop(stdout);
        assert_eq!(report["code"], expected, "{report}");
        assert_eq!(output.status.success(), expected == "matched", "{report}");
        assert_eq!(report["authenticated"], authenticated, "{report}");
        if authenticated {
            assert_eq!(report["stage"], "structure", "{report}");
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn actual_straight_matches_source_checkpoint() {
    Fixture::new("straight").check("matched");
}
#[test]
fn actual_if_else_matches_source_checkpoints() {
    Fixture::new("if_else").check("matched");
}
#[test]
fn actual_early_fallthrough_matches_source_checkpoints() {
    Fixture::new("early_fallthrough").check("matched");
}
#[test]
fn actual_first_match_preserves_ordered_exclusion() {
    Fixture::new("first_match_guarded").check("matched");
}
#[test]
fn missing_guard_is_rejected_after_authentication() {
    let f = Fixture::new("if_else");
    let mut c = f.commands(0);
    c.as_array_mut().unwrap().remove(2);
    f.replace(0, &c);
    f.check("checkpoint_mismatch");
}
#[test]
fn swapped_paths_are_rejected_after_authentication() {
    let f = Fixture::new("if_else");
    f.replace(0, &f.commands(1));
    f.replace(1, &f.commands(0));
    f.check("checkpoint_mismatch");
}
#[test]
fn duplicate_guard_with_unique_certificate_names_is_rejected() {
    let f = Fixture::new("if_else");
    f.replace(1, &f.commands(0));
    f.check("checkpoint_mismatch");
}
#[test]
fn omitted_path_is_rejected_after_authentication() {
    let f = Fixture::new("if_else");
    let mut m = f.manifest();
    m["obligations"]
        .as_array_mut()
        .unwrap()
        .retain(|entry| entry["idx"] != 1);
    f.save(&m);
    f.check("missing_obligation");
}
#[test]
fn extra_path_is_rejected_after_authentication() {
    let f = Fixture::new("if_else");
    let mut m = f.manifest();
    let mut entry = m["obligations"][0].clone();
    let raw = f
        .commands(0)
        .as_array()
        .unwrap()
        .iter()
        .map(render)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(f.root.join("certs/extra.smt2"), &raw).unwrap();
    entry["cert"] = json!("extra.smt2");
    entry["idx"] = json!(2);
    entry["sha256"] = json!(hash(raw.as_bytes()));
    entry["sig"] = json!(hex(&f.key.sign(raw.as_bytes()).to_bytes()));
    m["obligations"].as_array_mut().unwrap().push(entry);
    f.save(&m);
    f.check("extra_obligation");
}
#[test]
fn missing_first_match_exclusion_is_rejected() {
    let f = Fixture::new("first_match_guarded");
    let mut c = f.commands(2);
    c[2] = json!(["assert", "true"]);
    f.replace(2, &c);
    f.check("checkpoint_mismatch");
}
#[test]
fn equivalent_return_expression_is_a_checkpoint_deviation() {
    let f = Fixture::new("if_else");
    let mut c = f.commands(0);
    c[3] = json!(["assert", ["not", ["=", ["+", "x", "0"], "x"]]]);
    f.replace(0, &c);
    f.check("checkpoint_mismatch");
}
#[test]
fn extra_true_axiom_is_a_checkpoint_deviation() {
    let f = Fixture::new("if_else");
    let mut c = f.commands(0);
    c.as_array_mut()
        .unwrap()
        .insert(2, json!(["assert", "true"]));
    f.replace(0, &c);
    f.check("checkpoint_mismatch");
}
#[test]
fn parallel_let_values_use_outer_scope() {
    let f = Fixture::new("if_else");
    let mut c = f.commands(0);
    c[2] = json!([
        "assert",
        ["let", [["x", "0"], ["saved", "x"]], [">=", "saved", "0"]]
    ]);
    f.replace(0, &c);
    f.check("matched");
}
#[test]
fn inner_shadow_does_not_capture_saved_free_input() {
    let f = Fixture::new("if_else");
    let mut c = f.commands(0);
    c[2] = json!([
        "assert",
        [
            "let",
            [["saved", "x"]],
            ["let", [["x", "0"]], [">=", "saved", "0"]]
        ]
    ]);
    f.replace(0, &c);
    f.check("matched");
}
#[test]
fn changed_let_binding_is_rejected() {
    let f = Fixture::new("if_else");
    let mut c = f.commands(0);
    c[2] = json!([
        "assert",
        ["let", [["saved", "0"], ["x", "x"]], [">=", "saved", "0"]]
    ]);
    f.replace(0, &c);
    f.check("checkpoint_mismatch");
}
#[test]
fn quoted_identifier_alias_is_withheld_safely() {
    let f = Fixture::new("if_else");
    let mut c = f.commands(0);
    c[2] = json!(["assert", ["let", [["|x|", "0"]], [">=", "x", "0"]]]);
    f.replace(0, &c);
    f.check("unsupported_identifier");
}
#[test]
fn checker_authenticates_bundle_itself() {
    let f = Fixture::new("if_else");
    let other = SigningKey::generate(&mut rand_core::OsRng);
    std::fs::write(
        f.root.join("public.pem"),
        format!(
            "-----BEGIN ED25519 PUBLIC KEY-----\n{}\n-----END ED25519 PUBLIC KEY-----\n",
            hex(&other.verifying_key().to_bytes())
        ),
    )
    .unwrap();
    f.invoke("authentication_failed", false);
}

fn source_counterexample(name: &str) {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proof_correspondence");
    let expectations: Value = serde_json::from_slice(
        &std::fs::read(data.join("legacy-negative-expectations.json")).unwrap(),
    )
    .unwrap();
    let filename = format!("negative_{name}.rz");
    let expected = expectations["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["fixture"] == filename)
        .unwrap();
    let source = std::fs::read(data.join(&filename)).unwrap();
    assert_eq!(hash(&source), expected["source_sha256"]);
    let result = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "-t"])
        .arg(data.join(filename))
        .output()
        .unwrap();
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let record = json!({"case":std::thread::current().name(),"claim":"feasible source-input wrong-return control, not an equivalent checkpoint-policy mutation","source":String::from_utf8(source).unwrap(),"expected":expected,"exit":result.status.code(),"diagnostic":diagnostic,"compiler_sha256":hash(&std::fs::read(env!("CARGO_BIN_EXE_rz")).unwrap())});
    let raw = serde_json::to_vec(&record).unwrap();
    assert!(raw.len() < 256 * 1024);
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(b"CORRESPONDENCE_SOURCE_COUNTEREXAMPLE ")
        .unwrap();
    stdout.write_all(&raw).unwrap();
    stdout.write_all(b"\n").unwrap();
    drop(stdout);
    assert!(!result.status.success(), "wrong source accepted");
    assert!(
        diagnostic.contains("fn `wrong` violates `ensures"),
        "generic parser/type failure cannot satisfy source control: {diagnostic}"
    );
}
#[test]
fn genuine_straight_wrong_return_is_refuted() {
    source_counterexample("straight");
}
#[test]
fn genuine_wrong_else_return_is_refuted() {
    source_counterexample("if_else");
}
#[test]
fn genuine_wrong_fallthrough_return_is_refuted() {
    source_counterexample("early_fallthrough");
}
#[test]
fn genuine_wrong_match_arm_return_is_refuted() {
    source_counterexample("first_match_guarded");
}
