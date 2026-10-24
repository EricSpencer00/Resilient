//! String contents must never stand in for declaration keywords or delimiters.
#![cfg(unix)]
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
fn hash(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}
fn check(source: &str, import: bool, expected: &str) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "inventory_token_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    std::fs::write(root.join("program.rz"), source).unwrap();
    let mut files =
        vec![json!({"path":"program.rz","bytes":source.len(),"sha256":hash(source.as_bytes())})];
    let mut edges = vec![];
    if import {
        let helper = b"fn f(int x) -> int ensures result == x { return x; }\n";
        std::fs::write(root.join("helper.rz"), helper).unwrap();
        files.push(json!({"path":"helper.rz","bytes":helper.len(),"sha256":hash(helper)}));
        edges.push(json!({"from":"program.rz","specifier":"helper.rz","namespace":"dep","target":"helper.rz"}));
    }
    let graph =
        json!({"version":1,"entry":"program.rz","files":files,"imports":edges,"builtins":[]});
    let raw = serde_json::to_vec(&graph).unwrap();
    std::fs::write(root.join("graph.json"), &raw).unwrap();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let mut child = Command::new("python3")
        .arg(repo.join("agent-scripts/source-contract-inventory.py"))
        .arg("--source-root")
        .arg(&root)
        .arg("--graph")
        .arg(root.join("graph.json"))
        .arg("--graph-sha256")
        .arg(hash(&raw))
        .arg("--grammar")
        .arg(repo.join("resilient/tests/fixtures/source_inventory/grammar.json"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if started.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("token-kind control exceeded external watchdog");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    let report:Value=serde_json::from_slice(&output.stdout).unwrap_or_else(|_|json!({"raw":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)}));
    println!(
        "SOURCE_INVENTORY_TOKEN_KIND_CAPTURE {}",
        json!({"case":std::thread::current().name(),"source":source,"source_graph":graph,"expected":expected,"actual":report,"claim":"restricted grammar token-kind boundary, not full compiler syntax or proof validity"})
    );
    assert_eq!(report["code"], expected, "{output:?}");
    assert_eq!(output.status.success(), expected == "inventoried");
    if expected != "inventoried" {
        for key in ["contract_sites", "declarations", "files", "import_edges"] {
            assert!(
                report.get(key).is_none(),
                "partial inventory on token-kind rejection"
            );
        }
    } else {
        assert_eq!(report["contract_sites"].as_array().unwrap().len(), 1);
    }
}
macro_rules! rejects {
    ($name:ident,$source:literal) => {
        #[test]
        fn $name() {
            check($source, false, "unsupported_grammar");
        }
    };
}
rejects!(
    quoted_fn_is_not_a_keyword,
    r#""fn" f(int x) -> int ensures result == x { return x; }"#
);
rejects!(
    quoted_mod_is_not_a_keyword,
    r#""mod" m { fn f(int x) -> int ensures result == x { return x; } }"#
);
rejects!(
    quoted_impl_is_not_a_keyword,
    r#""impl" Cell { fn f(self) -> int ensures result == 0 { return 0; } }"#
);
rejects!(
    quoted_struct_is_not_a_keyword,
    r#""struct" Cell { int value }"#
);
rejects!(
    quoted_requires_is_not_a_keyword,
    r#"fn f(int x) -> int "requires" x >= 0 ensures result == x { return x; }"#
);
rejects!(
    quoted_ensures_is_not_a_keyword,
    r#"fn f(int x) -> int "ensures" result == x { return x; }"#
);
rejects!(
    quoted_parameter_delimiters_are_not_symbols,
    r#"fn f "(" int x ")" -> int ensures result == x { return x; }"#
);
rejects!(
    quoted_arrow_is_not_a_symbol,
    r#"fn f(int x) "->" int ensures result == x { return x; }"#
);
rejects!(
    quoted_body_opening_is_not_a_symbol,
    r#"fn f(int x) -> int ensures result == x "{" return x; }"#
);
rejects!(
    quoted_module_closing_is_not_a_symbol,
    r#"mod m { fn f(int x) -> int ensures result == x { return x; } "}""#
);
rejects!(
    quoted_contract_operator_is_not_a_symbol,
    r#"fn f(int x) -> int ensures result "==" x { return x; }"#
);
rejects!(
    quoted_contract_unary_is_not_a_symbol,
    r#"fn f(int x) -> int ensures "-" x == x { return x; }"#
);
rejects!(
    quoted_contract_delimiters_are_not_symbols,
    r#"fn f(int x) -> int ensures "(" result == x ")" { return x; }"#
);
#[test]
fn quoted_use_is_not_a_keyword() {
    check(r#""use" "helper.rz" as dep;"#, true, "unsupported_grammar");
}
#[test]
fn legitimate_body_string_remains_a_string() {
    check(
        r#"fn f(int x) -> int ensures result == x { let text = "fn mod impl ensures { }"; return x; }"#,
        false,
        "inventoried",
    );
}
