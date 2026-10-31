use std::{fs, process::Command};

fn check(name: &str, source: &str, success: bool, expected: &str) {
    let root =
        std::env::temp_dir().join(format!("vm_loop_invariant_{}_{}", std::process::id(), name));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("input.rz");
    fs::write(&path, source).unwrap();
    let mut results = Vec::new();
    for mode in ["interpreter", "match", "direct"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("RESILIENT_") {
                command.env_remove(key);
            }
        }
        command.arg("--no-cache").arg(&path);
        if mode != "interpreter" {
            command.arg("--vm").env("RESILIENT_DISPATCH", mode);
        }
        let output = command.output().unwrap();
        if let Some(records) = std::env::var_os("VM_LOOP_RECORDS") {
            let directory = std::path::PathBuf::from(records).join(name).join(mode);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("input.rz"), source).unwrap();
            fs::write(directory.join("stdout.txt"), &output.stdout).unwrap();
            fs::write(directory.join("stderr.txt"), &output.stderr).unwrap();
            fs::write(directory.join("status.txt"), output.status.to_string()).unwrap();
        }
        results.push((mode, output));
    }
    fs::remove_dir_all(root).unwrap();
    for (mode, output) in results {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.success(),
            success,
            "{name}/{mode}: {stdout}\n{stderr}"
        );
        assert_eq!(
            stdout
                .strip_suffix("Program executed successfully\n")
                .unwrap_or(&stdout),
            expected,
            "{name}/{mode}: {stderr}"
        );
        if !success {
            assert!(
                stderr.contains("loop invariant violated"),
                "{name}/{mode}: {stderr}"
            );
        }
    }
}

macro_rules! case {
    ($name:ident, $success:literal, $expected:literal, $source:literal) => {
        #[test]
        fn $name() {
            check(stringify!($name), $source, $success, $expected);
        }
    };
}

case!(
    while_entry,
    false,
    "",
    r#"let i = 0; i = -1; while i < 1 invariant i >= 0 { println("BODY"); i = i + 1; }"#
);
case!(
    while_zero,
    false,
    "",
    r#"while false invariant false { println("BODY"); }"#
);
case!(
    while_exit,
    false,
    "0\n",
    r#"let i = 0; while i < 1 invariant i == 0 { println(i); i = i + 1; }"#
);
case!(
    while_continue,
    false,
    "0\n",
    r#"let i = 0; while i < 2 invariant i < 1 { println(i); i = i + 1; continue; }"#
);
case!(
    while_break,
    true,
    "DONE\n",
    r#"let i = 0; while i < 1 invariant i == 0 { i = 1; break; } println("DONE");"#
);
case!(
    for_entry,
    false,
    "",
    r#"for item in [-1, 1] invariant item > 0 { println(item); }"#
);
case!(
    for_second,
    false,
    "1\n",
    r#"for item in [1, -1] invariant item > 0 { println(item); }"#
);
case!(
    for_continue,
    false,
    "1\n",
    r#"for item in [1, -1] invariant item > 0 { println(item); continue; }"#
);
case!(
    for_empty,
    true,
    "DONE\n",
    r#"for item in [] invariant false { println("BODY"); } println("DONE");"#
);
case!(
    for_shadow,
    true,
    "1\n2\n99\n",
    r#"let item = 99; for item in [1, 2] invariant item < 3 { println(item); } println(item);"#
);
case!(
    body_form,
    false,
    "",
    r#"let i = 0; i = -1; while i < 1 { println("BODY"); invariant i >= 0; i = i + 1; }"#
);
case!(
    function_while,
    false,
    "",
    r#"fn check(int reading) { while reading < 1 invariant reading >= 0 { println("BODY"); reading = reading + 1; } } check(-1);"#
);
case!(
    function_for,
    false,
    "1\n",
    r#"fn check() { for item in [1, -1] invariant item > 0 { println(item); } } check();"#
);
case!(
    clause_order,
    false,
    "FIELD\n",
    r#"fn gate(bool ok, string label) -> bool { println(label); return ok; } while false invariant gate(false, "FIELD") { invariant gate(true, "BODYINV"); println("BODY"); }"#
);
case!(
    while_valid_trace,
    true,
    "INV\n0\nINV\n1\nINV\n",
    r#"fn gate() -> bool { println("INV"); return true; } let i = 0; while i < 2 invariant gate() { println(i); i = i + 1; }"#
);
case!(
    for_body_form,
    false,
    "1\n",
    r#"for item in [1, -1] { invariant item > 0; println(item); }"#
);

case!(
    closure_while,
    false,
    "",
    r#"let bound = 0; let f = fn() -> int { let i = 0; while i < 1 invariant i < bound { println("BODY"); i = i + 1; } return i; }; println(f());"#
);
case!(
    closure_for,
    false,
    "",
    r#"let bound = 0; let f = fn() -> int { for item in [1] invariant item < bound { println("BODY"); } return 0; }; println(f());"#
);
case!(
    closure_body,
    false,
    "",
    r#"let bound = 0; let f = fn() -> int { let i = 0; while i < 1 { invariant i < bound; println("BODY"); i = i + 1; } return i; }; println(f());"#
);
case!(
    closure_valid,
    true,
    "1\n",
    r#"let bound = 2; let f = fn() -> int { let i = 0; while i < 1 invariant i < bound { i = i + 1; } return i; }; println(f());"#
);
