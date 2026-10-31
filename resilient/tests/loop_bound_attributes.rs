use std::{fs, process::Command};

const INVALID: &[&str] = &[
    "",
    "()",
    "(0)",
    "(-1)",
    "(4294967296)",
    "(1.0)",
    "(1.5)",
    "(1e0)",
    "(true)",
    "(\"1\")",
    "(n)",
    "(1+1)",
    "((1))",
    "(1,2)",
    "(1,)",
    "(bound=1)",
];

fn assert_rejected(source: &str, message: &str) {
    let result = resilient::run_program(source);
    assert!(
        result.errors.iter().any(|e| e.contains(message)),
        "{source}: {:?}",
        result.errors
    );
    assert!(!result.ok, "{source}: {:?}", result.errors);
    assert!(result.stdout.is_empty(), "{source}: {}", result.stdout);
}

#[test]
fn malformed_values_are_not_silently_discarded_or_coerced() {
    for args in INVALID {
        assert_rejected(
            &format!("#[loop_bound{args}] fn f() {{println(99);}} f();"),
            "#[loop_bound] requires exactly one integer literal",
        );
    }
}

#[test]
fn unsupported_targets_and_missing_declarations_are_rejected() {
    for item in [
        "let f=fn(){println(99);}; f();",
        "struct S {int value}",
        "println(99);",
        "",
    ] {
        assert_rejected(
            &format!("#[loop_bound(1)] {item}"),
            "requires a named function",
        );
    }
}

#[test]
fn conflicts_are_rejected_in_both_orders_and_at_nested_scope() {
    for bounds in [
        "#[loop_bound(1)] #[loop_bound(2)]",
        "#[loop_bound(2)] #[loop_bound(1)]",
    ] {
        assert_rejected(
            &format!("{bounds} fn f(){{println(99);}} f();"),
            "conflicting #[loop_bound]",
        );
        assert_rejected(
            &format!("fn outer(){{{bounds} fn f(){{println(99);}} f();}} outer();"),
            "conflicting #[loop_bound]",
        );
    }
}

#[test]
fn valid_boundaries_repeated_bounds_and_cfg_composition_work() {
    for attributes in [
        "#[loop_bound(1)]",
        "#[loop_bound(4294967295)]",
        "#[loop_bound(1)] #[loop_bound(1)]",
        "#[loop_bound(1)] #[cfg(not(feature=\"never_enabled_schema_test\"))]",
        "#[cfg(not(feature=\"never_enabled_schema_test\"))] #[loop_bound(1)]",
    ] {
        let source = format!("{attributes} fn f(){{let i=0;while i<1{{println(i);i=i+1;}}}} f();");
        let result = resilient::run_program(&source);
        assert!(result.ok, "{source}: {:?}", result.errors);
        assert_eq!(result.stdout.trim(), "0");
    }
    let result = resilient::run_program(
        "#[loop_bound(1)] #[cfg(feature=\"never_enabled_schema_test\")] fn f(){} println(7);",
    );
    assert!(result.ok, "{:?}", result.errors);
    assert_eq!(result.stdout.trim(), "7");
}

#[test]
fn cli_rejects_local_and_imported_invalid_bounds_before_any_side_effect() {
    let root = std::env::temp_dir().join(format!("loop_bound_schema_{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut cases: Vec<String> = INVALID
        .iter()
        .map(|args| format!("#[loop_bound{args}] fn f(){{println(99);}} f();"))
        .collect();
    cases.push("#[loop_bound(1)] #[loop_bound(2)] fn f(){println(99);} f();".into());
    cases.push("#[loop_bound(1)] let f=fn(){println(99);};f();".into());
    fs::write(
        root.join("bad.rz"),
        "#[loop_bound(0)] pub fn f(){println(99);}",
    )
    .unwrap();
    cases.push("use \"bad.rz\" as bad; bad::f();".into());
    let mut modes = vec!["interpreter", "match", "direct"];
    if cfg!(feature = "jit") {
        modes.push("jit");
    }
    for (index, source) in cases.iter().enumerate() {
        let path = root.join(format!("case{index}.rz"));
        fs::write(&path, source).unwrap();
        for mode in &modes {
            for strict in [false, true] {
                let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
                for (key, _) in std::env::vars_os() {
                    if key.to_string_lossy().starts_with("RESILIENT_") {
                        command.env_remove(key);
                    }
                }
                command.arg(&path);
                if strict {
                    command.arg("--typecheck-strict");
                }
                if *mode == "jit" {
                    command.arg("--jit");
                } else if *mode != "interpreter" {
                    command.arg("--vm").env("RESILIENT_DISPATCH", mode);
                }
                let output = command.output().unwrap();
                assert!(
                    !output.status.success(),
                    "{source}/{mode}/{strict}: {output:?}"
                );
                assert!(
                    output.stdout.is_empty(),
                    "{source}/{mode}/{strict}: {output:?}"
                );
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("loop_bound"),
                    "{source}/{mode}/{strict}: {output:?}"
                );
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}
