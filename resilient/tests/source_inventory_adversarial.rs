//! Public negative controls for the restricted inventory's trust/resource boundary.
#![cfg(unix)]
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
fn hash(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}
struct Fixture {
    root: PathBuf,
    graph: Value,
    graph_hash: Option<String>,
    raw_graph: Option<Vec<u8>>,
    graph_fifo: bool,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "inventory_boundary_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let mut f = Self {
            root,
            graph: json!({"version":1,"entry":"program.rz","files":[],"imports":[],"builtins":[]}),
            graph_hash: None,
            raw_graph: None,
            graph_fifo: false,
        };
        f.file(
            "program.rz",
            b"fn f(int x) -> int ensures result == x { return x; }\n",
        );
        f
    }
    fn file(&mut self, name: &str, raw: &[u8]) {
        std::fs::write(self.root.join(name), raw).unwrap();
        let record = json!({"path":name,"bytes":raw.len(),"sha256":hash(raw)});
        let files = self.graph["files"].as_array_mut().unwrap();
        if let Some(file) = files.iter_mut().find(|f| f["path"] == name) {
            *file = record;
        } else {
            files.push(record);
        }
    }
    fn check(&self, expected: &str) {
        let graph_path = self.root.join("graph.json");
        let raw = self
            .raw_graph
            .clone()
            .unwrap_or_else(|| serde_json::to_vec(&self.graph).unwrap());
        if self.graph_fifo {
            assert!(
                Command::new("mkfifo")
                    .arg(&graph_path)
                    .status()
                    .unwrap()
                    .success()
            );
        } else {
            std::fs::write(&graph_path, &raw).unwrap();
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let started = Instant::now();
        let mut child = Command::new("python3")
            .arg(repo.join("agent-scripts/source-contract-inventory.py"))
            .arg("--source-root")
            .arg(&self.root)
            .arg("--graph")
            .arg(&graph_path)
            .arg("--graph-sha256")
            .arg(self.graph_hash.clone().unwrap_or_else(|| hash(&raw)))
            .arg("--grammar")
            .arg(repo.join("resilient/tests/fixtures/source_inventory/grammar.json"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if started.elapsed() > Duration::from_secs(5) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("negative input did not reject within external watchdog");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        let report:Value=serde_json::from_slice(&output.stdout).unwrap_or_else(|_|json!({"raw":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)}));
        {
            let mut records_stdout = std::io::stdout().lock();
            writeln!(records_stdout,
                        "SOURCE_INVENTORY_BOUNDARY_CAPTURE {}",
                        json!({"case":std::thread::current().name(),"expected":expected,"actual":report,"graph_sha256":hash(&raw),"elapsed_ms":started.elapsed().as_millis(),"claim":"public adversarial lexical/sourcegraph boundary; no compiler proof or signature acceptance"})
                    ).unwrap();
            records_stdout.flush().unwrap();
        }
        assert!(!output.status.success(), "{output:?}");
        assert_eq!(report["code"], expected);
        for key in ["contract_sites", "declarations", "files", "import_edges"] {
            assert!(
                report.get(key).is_none(),
                "partial inventory on rejection: {report}"
            );
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn trusted_graph_hash_mismatch() {
    let mut f = Fixture::new();
    f.graph_hash = Some("0".repeat(64));
    f.check("graph_hash_mismatch");
}
#[test]
fn source_hash_mismatch() {
    let f = Fixture::new();
    std::fs::write(f.root.join("program.rz"), b" ").unwrap();
    f.check("source_hash_mismatch");
}
#[test]
fn duplicate_json_key() {
    let mut f = Fixture::new();
    let raw = serde_json::to_string(&f.graph).unwrap();
    f.raw_graph = Some(format!("{{\"version\":1,{}", &raw[1..]).into_bytes());
    f.check("invalid_graph");
}
#[test]
fn duplicate_graph_file() {
    let mut f = Fixture::new();
    let file = f.graph["files"][0].clone();
    f.graph["files"].as_array_mut().unwrap().push(file);
    f.check("invalid_graph");
}
#[test]
fn missing_graph_file() {
    let f = Fixture::new();
    std::fs::remove_file(f.root.join("program.rz")).unwrap();
    f.check("invalid_input");
}
#[test]
fn unreachable_graph_file() {
    let mut f = Fixture::new();
    f.file("orphan.rz", b"fn orphan() { }\n");
    f.check("invalid_graph");
}
#[test]
fn extra_graph_edge() {
    let mut f = Fixture::new();
    f.graph["imports"] = json!([{"from":"program.rz","specifier":"program.rz","namespace":null,"target":"program.rz"}]);
    f.check("import_graph_mismatch");
}
#[test]
fn traversal_graph_path() {
    let mut f = Fixture::new();
    f.graph["files"][0]["path"] = json!("../outside.rz");
    f.check("invalid_graph");
}
#[test]
fn unterminated_string() {
    let mut f = Fixture::new();
    f.file("program.rz", b"fn f() { let text = \"unfinished; }");
    f.check("invalid_lexical_input");
}
#[test]
fn unsupported_string_escape() {
    let mut f = Fixture::new();
    f.file("program.rz", b"fn f() { let text = \"\\q\"; }");
    f.check("invalid_lexical_input");
}
#[test]
fn nested_comment() {
    let mut f = Fixture::new();
    f.file("program.rz", b"/* outer /* nested */ end */ fn f() { }");
    f.check("invalid_lexical_input");
}
#[test]
fn file_byte_bound() {
    let mut f = Fixture::new();
    f.file("program.rz", &vec![b' '; 131073]);
    f.check("invalid_graph");
}
#[test]
fn graph_file_count_bound() {
    let mut f = Fixture::new();
    for index in 0..32 {
        f.file(&format!("f{index}.rz"), b" ");
    }
    f.check("invalid_graph");
}
#[test]
fn total_source_byte_bound() {
    let mut f = Fixture::new();
    f.file("program.rz", &vec![b' '; 40000]);
    for index in 0..26 {
        f.file(&format!("f{index}.rz"), &vec![b' '; 40000]);
    }
    f.check("resource_limit");
}
#[test]
fn whole_graph_token_bound() {
    let mut f = Fixture::new();
    let body = ";".repeat(40000);
    f.file(
        "program.rz",
        format!("use \"other.rz\"; fn f() {{{body}}}").as_bytes(),
    );
    f.file("other.rz", format!("fn f() {{{body}}}").as_bytes());
    f.graph["imports"] =
        json!([{"from":"program.rz","specifier":"other.rz","namespace":null,"target":"other.rz"}]);
    f.check("resource_limit");
}
#[test]
fn declaration_depth_bound() {
    let mut f = Fixture::new();
    let mut s = String::new();
    for index in 0..34 {
        s.push_str(&format!("mod m{index} {{ "));
    }
    s.push_str("fn f() { }");
    s.push_str(&"}".repeat(34));
    f.file("program.rz", s.as_bytes());
    f.check("resource_limit");
}
#[test]
fn body_depth_bound() {
    let mut f = Fixture::new();
    let s = format!("fn f() {{ {} {} }}", "{".repeat(34), "}".repeat(34));
    f.file("program.rz", s.as_bytes());
    f.check("resource_limit");
}
#[test]
fn symlink_source_input() {
    let f = Fixture::new();
    std::fs::rename(f.root.join("program.rz"), f.root.join("real.rz")).unwrap();
    std::os::unix::fs::symlink("real.rz", f.root.join("program.rz")).unwrap();
    f.check("invalid_graph");
}
#[test]
fn fifo_source_input_rejects_without_waiting() {
    let f = Fixture::new();
    std::fs::remove_file(f.root.join("program.rz")).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(f.root.join("program.rz"))
            .status()
            .unwrap()
            .success()
    );
    f.check("resource_limit");
}
#[test]
fn fifo_graph_input_rejects_without_waiting() {
    let mut f = Fixture::new();
    f.graph_fifo = true;
    f.check("resource_limit");
}
