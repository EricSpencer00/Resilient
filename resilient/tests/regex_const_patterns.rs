use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_source(name: &str, source: &str) -> Output {
    let path = std::env::temp_dir().join(format!(
        "res_regex_const_{}_{}.rz",
        std::process::id(),
        name
    ));
    std::fs::write(&path, source).expect("write regex source");
    let output = Command::new(bin())
        .arg("--typecheck-strict")
        .arg(&path)
        .output()
        .expect("spawn rz");
    let _ = std::fs::remove_file(path);
    output
}

#[test]
fn invalid_const_pattern_is_rejected_before_a_cold_branch_runs() {
    let output = run_source(
        "invalid",
        r#"
const PATTERN = "[invalid";

fn main() {
    let run_branch = false;
    if run_branch {
        println(regex_match("abc", PATTERN));
    }
    println("done");
}

main();
"#,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "invalid const pattern unexpectedly passed:\nstdout={}\nstderr={stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        stderr.contains("regex_match"),
        "diagnostic should name regex_match: {stderr}"
    );
    assert!(
        stderr.contains("invalid regex pattern"),
        "diagnostic should identify the invalid pattern: {stderr}"
    );
    assert!(
        stderr.contains(":7:"),
        "diagnostic should point at the call-site line: {stderr}"
    );
}

#[test]
fn valid_const_alias_and_string_concatenation_still_run() {
    let output = run_source(
        "valid",
        r#"
const PREFIX = "[a-z";
const PATTERN = PREFIX + "]+";

fn main() {
    println(regex_match("abc", PATTERN));
}

main();
"#,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "valid const pattern failed:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("true"),
        "expected the compiled regex to match: {stdout}"
    );
}

#[test]
fn non_const_binding_remains_outside_static_pattern_analysis() {
    let output = run_source(
        "let",
        r#"
fn main() {
    let pattern = "[a-z]+";
    println(regex_match("abc", pattern));
}

main();
"#,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "non-const binding should remain valid:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("true"),
        "expected a successful match: {stdout}"
    );
}
