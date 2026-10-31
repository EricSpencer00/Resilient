use std::{fs, process::Command};

#[test]
fn imported_bounds_belong_to_the_selected_declaration() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/loop_bound_imports.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let root =
            std::env::temp_dir().join(format!("loop_bound_imports_{}_{name}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        for (path, source) in case["files"].as_object().unwrap() {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source.as_str().unwrap()).unwrap();
        }
        let mut modes = vec!["interpreter", "match", "direct"];
        if cfg!(feature = "jit") {
            modes.push("jit");
        }
        for mode in modes {
            let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("RESILIENT_") {
                    command.env_remove(key);
                }
            }
            command.current_dir(&root).arg(root.join("entry.rz"));
            if let Some(flags) = case["flags"].as_array() {
                for flag in flags {
                    command.arg(flag.as_str().unwrap());
                }
            }
            if mode == "jit" {
                command.arg("--jit");
            } else if mode != "interpreter" {
                command.arg("--vm").env("RESILIENT_DISPATCH", mode);
            }
            let output = command.output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let success = case["success"].as_bool().unwrap();
            assert_eq!(
                output.status.success(),
                success,
                "{name}/{mode}: {output:?}"
            );
            assert_eq!(
                stdout
                    .trim_end()
                    .trim_end_matches("Program executed successfully")
                    .trim_end(),
                case["stdout"].as_str().unwrap(),
                "{name}/{mode}: {output:?}"
            );
            if !success {
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains(case["error"].as_str().unwrap_or("loop bound exceeded")),
                    "{name}/{mode}: {output:?}"
                );
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn static_verification_uses_imported_declaration_ownership() {
    let root =
        std::env::temp_dir().join(format!("loop_bound_import_static_{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("entry.rz"),
        "@require_contracts(strict)\nuse \"dep.rz\" as dep;",
    )
    .unwrap();
    fs::write(root.join("dep.rz"),"#[loop_bound(2)] fn f(int n)->int requires n>=0 requires n<=2 ensures result>=0 {let i=0;while i<n {i=i+1;}return i;}").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg("--typecheck")
        .arg(root.join("entry.rz"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("requires #[loop_bound"),
        "{output:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn in_memory_nested_functions_keep_distinct_annotations() {
    let output = resilient::run_program(
        "fn left(){#[loop_bound(1)] fn g(){let i=0;while i<2 {println(i);i=i+1;}} g();} fn right(){#[loop_bound(3)] fn g(){let i=0;while i<2 {println(i);i=i+1;}} g();} left();",
    );
    assert!(!output.ok, "{:?}", output.errors);
    assert_eq!(output.stdout, "0\n");
    assert!(output.errors.join("\n").contains("loop bound exceeded"));
}
