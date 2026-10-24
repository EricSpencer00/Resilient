//! Independent inventory requirements are frozen separately from compiler output.
#![cfg(unix)]
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
struct Fixture {
    root: PathBuf,
    expected: Value,
}
impl Fixture {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let data = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/source_inventory")
            .join(name);
        let expected: Value =
            serde_json::from_slice(&std::fs::read(data.join("expected.json")).unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!(
            "source_inventory_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        for file in expected["files"].as_array().unwrap() {
            let path = file["path"].as_str().unwrap();
            std::fs::copy(data.join(path), root.join(path)).unwrap();
        }
        Self { root, expected }
    }
    fn run(&self, code: &str) -> Value {
        let mut files = self.expected["files"].clone();
        for file in files.as_array_mut().unwrap() {
            let raw = std::fs::read(self.root.join(file["path"].as_str().unwrap())).unwrap();
            file["bytes"] = json!(raw.len());
            file["sha256"] = json!(hash(&raw));
        }
        let graph = json!({"version":1,"entry":"program.rz","files":files,"imports":self.expected["import_edges"],"builtins":[]});
        let raw = serde_json::to_vec(&graph).unwrap();
        let graph_path = self.root.join("graph.json");
        std::fs::write(&graph_path, &raw).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let output = Command::new("python3")
            .arg(repo.join("agent-scripts/source-contract-inventory.py"))
            .arg("--source-root")
            .arg(&self.root)
            .arg("--graph")
            .arg(&graph_path)
            .arg("--graph-sha256")
            .arg(hash(&raw))
            .arg("--grammar")
            .arg(repo.join("resilient/tests/fixtures/source_inventory/grammar.json"))
            .output()
            .unwrap();
        let report:Value=serde_json::from_slice(&output.stdout).unwrap_or_else(|_|json!({"unavailable":String::from_utf8_lossy(&output.stderr),"raw":String::from_utf8_lossy(&output.stdout)}));
        let record = json!({"case":std::thread::current().name(),"expected_code":code,"expected_inventory":self.expected,"actual_report":report,"source_graph":graph,"claim":"lexical enumeration only; no compiler/signature/proof acceptance observed"});
        let record = serde_json::to_string(&record).unwrap();
        assert!(record.len() <= 256 * 1024);
        println!("SOURCE_INVENTORY_REQUIREMENT_CAPTURE {record}");
        assert_eq!(report["code"], code, "{output:?}");
        assert_eq!(output.status.success(), code == "inventoried");
        report
    }
    fn check(&self) {
        let report = self.run("inventoried");
        let mut declarations = self.expected["declarations"].clone();
        for declaration in declarations.as_array_mut().unwrap() {
            declaration
                .as_object_mut()
                .unwrap()
                .remove("body_features_for_review");
        }
        let mut sites = self.expected["contract_sites"].clone();
        for site in sites.as_array_mut().unwrap() {
            site.as_object_mut().unwrap().remove("selected");
            site.as_object_mut().unwrap().remove("required_policy");
        }
        assert_eq!(report["declarations"], declarations);
        assert_eq!(report["contract_sites"], sites);
        assert_eq!(report["import_edges"], self.expected["import_edges"]);
        assert_eq!(report["files"], self.expected["files"]);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
macro_rules! inventory {
    ($name:ident,$case:literal) => {
        #[test]
        fn $name() {
            Fixture::new($case).check();
        }
    };
}
inventory!(top_level_site, "top_level");
inventory!(same_name_module_sites, "same_name_modules");
inventory!(physical_import_once_with_both_alias_edges, "import_aliases");
inventory!(method_and_free_function_do_not_collide, "method_and_free");
inventory!(shadowed_body_still_has_declared_contract, "shadowed_input");
inventory!(
    unsupported_call_still_has_declared_contract,
    "unsupported_call"
);
inventory!(vacuous_entry_still_has_both_contract_sites, "vacuous_entry");
inventory!(non_result_clause_is_not_omitted, "non_result_clause");
inventory!(
    unresolved_call_still_has_declared_contract,
    "unresolved_call"
);
inventory!(
    comment_and_string_decoys_are_not_sites,
    "comment_string_decoys"
);
#[test]
fn executable_entry_rejects_whole_inventory() {
    let f = Fixture::new("top_level");
    let p = f.root.join("program.rz");
    let mut s = std::fs::read_to_string(&p).unwrap();
    s.push_str("println(1);\n");
    std::fs::write(p, s).unwrap();
    let r = f.run("unsupported_grammar");
    assert!(r.get("contract_sites").is_none());
}
#[test]
fn nested_declaration_rejects_whole_inventory() {
    let f = Fixture::new("top_level");
    std::fs::write(
        f.root.join("program.rz"),
        "fn outer() { fn inner(int x) -> int ensures result == x { return x; } }\n",
    )
    .unwrap();
    let r = f.run("unsupported_grammar");
    assert!(r.get("contract_sites").is_none());
}
#[test]
fn unknown_pub_syntax_is_not_silently_skipped() {
    let f = Fixture::new("top_level");
    let p = f.root.join("program.rz");
    let s = std::fs::read_to_string(&p).unwrap();
    std::fs::write(p, format!("pub {s}")).unwrap();
    f.run("unsupported_grammar");
}
#[test]
fn duplicate_declaration_rejects_inventory() {
    let f = Fixture::new("top_level");
    let p = f.root.join("program.rz");
    let s = std::fs::read_to_string(&p).unwrap();
    std::fs::write(p, format!("{s}{s}")).unwrap();
    f.run("duplicate_declaration");
}
#[test]
fn missing_alias_edge_rejects_complete_graph_claim() {
    let mut f = Fixture::new("import_aliases");
    f.expected["import_edges"].as_array_mut().unwrap().pop();
    f.run("import_graph_mismatch");
}
#[test]
fn malformed_comment_rejects_whole_inventory() {
    let f = Fixture::new("top_level");
    let p = f.root.join("program.rz");
    let s = std::fs::read_to_string(&p).unwrap();
    std::fs::write(p, format!("{s}/* unfinished")).unwrap();
    f.run("invalid_lexical_input");
}
