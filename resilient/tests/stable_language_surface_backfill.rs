use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_file(tag: &str, body: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("res_3128_{}_{}_{}.rz", tag, std::process::id(), n));
    std::fs::write(&path, body).expect("write scratch file");
    path
}

fn run_source(tag: &str, body: &str) -> (String, String, Option<i32>) {
    let path = tmp_file(tag, body);
    let output = Command::new(bin()).arg(&path).output().expect("spawn rz");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn meaningful_stdout_lines(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|line| !line.is_empty() && *line != "Program executed successfully")
        .collect()
}

#[test]
fn older_control_flow_and_operator_surface_executes_end_to_end() {
    let src = r#"
/* RES-3128: direct smoke for older stable parser/runtime constructs. */
fn call_before_decl(int n) {
    return helper(n);
}

fn helper(int n) {
    return -n + 12;
}

fn main() {
    let i = 0;
    let total = 0;
    while i < 4 {
        total = total + i;
        i = i + 1;
    }

    println(total);
    println(call_before_decl(5));
    println(0x1f + 0b10_10);
    println(5 % 3);
    println(!false && true);
    println(((0b1010 & 0b1100) | 0b0011) ^ 0b0101);
}

main();
"#;

    let (stdout, stderr, code) = run_source("legacy_control_flow", src);
    assert_eq!(
        code,
        Some(0),
        "legacy stable surface should run cleanly; stdout={stdout} stderr={stderr}"
    );

    let lines = meaningful_stdout_lines(&stdout);
    assert_eq!(lines, vec!["6", "7", "41", "2", "true", "14"]);
}

#[test]
fn static_let_string_ops_and_bare_return_have_direct_smoke_coverage() {
    let src = r#"
static let hits = 0;

fn bump() {
    hits = hits + 1;
    return;
}

fn main() {
    bump();
    bump();

    println(hits);
    println(len("gear"));

    if "alpha" < "beta" && "gamma" >= "gamma" {
        println("cmp-ok");
    } else {
        println("cmp-bad");
    }
}

main();
"#;

    let (stdout, stderr, code) = run_source("legacy_static_and_strings", src);
    assert_eq!(
        code,
        Some(0),
        "static/string stable surface should run cleanly; stdout={stdout} stderr={stderr}"
    );

    let lines = meaningful_stdout_lines(&stdout);
    assert_eq!(lines, vec!["2", "4", "cmp-ok"]);
}
