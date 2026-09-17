use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_source(name: &str, source: &str) -> Output {
    let path = std::env::temp_dir().join(format!(
        "res_probabilistic_{}_{}.rz",
        std::process::id(),
        name
    ));
    std::fs::write(&path, source).expect("write probabilistic source");
    let output = Command::new(bin())
        .arg("--typecheck-strict")
        .arg(&path)
        .output()
        .expect("spawn rz");
    let _ = std::fs::remove_file(path);
    output
}

fn function_with_attribute(attribute: &str) -> String {
    format!(
        "{attribute}\n\
         fn sample(int x) -> int {{ return x + 1; }}\n\
         fn main() {{ println(sample(1)); }}\n\
         main();\n"
    )
}

#[test]
fn valid_probabilistic_attribute_is_reachable_from_source() {
    let output = run_source(
        "valid",
        &function_with_attribute(r#"#[probabilistic(clause = "result > 0", p = "0.99")]"#),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "valid probabilistic declaration failed:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("2"),
        "expected the annotated function to run: {stdout}"
    );
}

#[test]
fn malformed_probabilistic_attributes_are_rejected_with_diagnostics() {
    let cases = [
        (
            "bad-number",
            r#"#[probabilistic(clause = "result > 0", p = "abc")]"#,
            "decimal probability",
        ),
        (
            "out-of-range",
            r#"#[probabilistic(clause = "result > 0", p = "1.5")]"#,
            "in [0.0, 1.0]",
        ),
        (
            "missing-p",
            r#"#[probabilistic(clause = "result > 0")]"#,
            "requires a p argument",
        ),
        (
            "missing-clause",
            r#"#[probabilistic(p = "0.9")]"#,
            "requires a clause argument",
        ),
        (
            "unknown-key",
            r#"#[probabilistic(clause = "result > 0", probability = "0.9")]"#,
            "unknown argument probability",
        ),
    ];

    for (name, attribute, expected) in cases {
        let output = run_source(name, &function_with_attribute(attribute));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "{name} unexpectedly passed:\nstdout={}\nstderr={stderr}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            stderr.contains("error[probabilistic]"),
            "{name} missing probabilistic diagnostic: {stderr}"
        );
        assert!(
            stderr.contains("sample"),
            "{name} diagnostic should name the item: {stderr}"
        );
        assert!(
            stderr.contains(expected),
            "{name} diagnostic missing {expected:?}: {stderr}"
        );
    }
}

#[test]
fn duplicate_probabilistic_attributes_are_rejected() {
    let source = r#"
#[probabilistic(clause = "result > 0", p = "0.9")]
#[probabilistic(clause = "result > 0", p = "0.8")]
fn sample(int x) -> int { return x + 1; }
fn main() { println(sample(1)); }
main();
"#;
    let output = run_source("duplicate", source);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "duplicate declaration unexpectedly passed:\n{stderr}"
    );
    assert!(stderr.contains("duplicate probabilistic contract"));
    assert!(stderr.contains("sample"));
    assert!(stderr.contains("first declared at line"));
}
