use std::process::Command;

const OVERFLOWS: &[&str] = &[
    "9223372036854775808",
    "9_223_372_036_854_775_808",
    "0x8000000000000000",
    "0o1000000000000000000000",
    "0b1000000000000000000000000000000000000000000000000000000000000000",
];

#[test]
fn library_rejects_overflow_before_any_execution() {
    for literal in OVERFLOWS {
        let result = resilient::run_program(&format!(
            "println(\"FORBIDDEN\");\nlet value = {literal};\nprintln(value);"
        ));
        assert!(!result.ok, "accepted overflowing literal {literal}");
        assert!(
            result.stdout.is_empty(),
            "executed before rejecting {literal}"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains("2:13:") && e.contains("overflows i64")),
            "{literal}: {:?}",
            result.errors
        );
    }
}

#[test]
fn cli_rejects_overflow_in_both_execution_backends() {
    let root =
        std::env::temp_dir().join(format!("integer-literal-rejection-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("input.rz");
    for literal in OVERFLOWS {
        std::fs::write(
            &source,
            format!("println(\"FORBIDDEN\");\nlet value = {literal};\nprintln(value);"),
        )
        .unwrap();
        for backend in ["--no-typecheck", "--vm"] {
            let output = Command::new(env!("CARGO_BIN_EXE_rz"))
                .args(["--no-cache", backend])
                .arg(&source)
                .output()
                .unwrap();
            assert!(
                !output.status.success(),
                "{literal} {backend}: accepted overflow"
            );
            assert!(!String::from_utf8_lossy(&output.stdout).contains("FORBIDDEN"));
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("2:13:") && stderr.contains("overflows i64"),
                "{stderr}"
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn representable_integer_boundaries_and_literal_text_still_execute() {
    for literal in [
        "9223372036854775807",
        "0x7fffffffffffffff",
        "0o777777777777777777777",
        "0b111111111111111111111111111111111111111111111111111111111111111",
    ] {
        let result = resilient::run_program(&format!("println({literal});"));
        assert!(result.ok, "{literal}: {:?}", result.errors);
        assert_eq!(result.stdout.trim(), "9223372036854775807");
    }
    let result =
        resilient::run_program("// 9223372036854775808\nprintln(\"9223372036854775808\");");
    assert!(result.ok, "{:?}", result.errors);
    assert_eq!(result.stdout.trim(), "9223372036854775808");
}

#[test]
fn overflow_diagnostics_keep_unicode_columns_and_remain_bounded() {
    let result = resilient::run_program("let text = \"λ\"; let value = 9223372036854775808;");
    assert!(!result.ok);
    assert!(
        result.errors.iter().any(|e| e.contains("1:29:")),
        "{:?}",
        result.errors
    );
    let source = "let value = 9223372036854775808;\n".repeat(300);
    let result = resilient::run_program(&source);
    assert!(!result.ok);
    assert!(!result.errors.is_empty() && result.errors.len() <= 100);
}

#[test]
fn physical_origins_reject_recovered_overflow() {
    let path = std::env::temp_dir().join(format!("overflow-origins-{}.rz", std::process::id()));
    std::fs::write(&path, "fn value() -> int { return 9223372036854775808; }").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg("source-origins")
        .arg(&path)
        .args([
            "--source-key",
            "overflow",
            "--profile",
            "physical-origins-v1",
        ])
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        !output.status.success(),
        "accepted invalid physical source: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
