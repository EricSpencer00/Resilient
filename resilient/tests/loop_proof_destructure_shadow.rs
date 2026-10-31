use std::{fs, process::Command};

fn check_case(name: &str, source: &str, provable: bool) {
    let root = std::env::temp_dir().join(format!("loop_shadow_{}_{name}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("input.rz");
    fs::write(&path, format!("println(\"EXECUTION_STARTED\"); {source}")).unwrap();
    for mode in ["interpreter", "match", "direct"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("RESILIENT_") {
                command.env_remove(key);
            }
        }
        command
            .args(["--deny-unproven-invariants", "--verbose"])
            .arg(&path);
        if mode != "interpreter" {
            command.arg("--vm").env("RESILIENT_DISPATCH", mode);
        }
        let output = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let success = provable && cfg!(feature = "z3");
        assert_eq!(
            output.status.success(),
            success,
            "{name}/{mode}: {output:?}"
        );
        assert_eq!(
            stdout.contains("EXECUTION_STARTED"),
            success,
            "{name}/{mode}: {output:?}"
        );
        assert_eq!(
            stderr.contains("invariant proven"),
            success,
            "{name}/{mode}: {output:?}"
        );
        if !success {
            assert!(
                stderr.contains("unproven-loop-invariant:"),
                "{name}/{mode}: {output:?}"
            );
            assert!(
                !stderr.contains("Runtime error"),
                "{name}/{mode}: {output:?}"
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tuple_shadow_cannot_reuse_constant() {
    check_case(
        "tuple",
        "let x=0; let (x,y)=(1,2); while false invariant x==0 {}",
        false,
    );
    check_case(
        "second",
        "let x=0; let (y,x)=(2,1); while false invariant x==0 {}",
        false,
    );
}

#[test]
fn struct_shadow_uses_local_binding_names() {
    check_case(
        "struct",
        "struct Pair { int x, } let x=0; let Pair {x}=new Pair {x:1}; while false invariant x==0 {}",
        false,
    );
    check_case(
        "alias",
        "struct Pair { int value, } let x=0; let Pair {value:x}=new Pair {value:1}; while false invariant x==0 {}",
        false,
    );
}

#[test]
fn function_local_and_nested_loop_shadows_cannot_reuse_constants() {
    check_case(
        "function",
        "fn f(){let x=0; let (x,y)=(1,2); while false invariant x==0 {}} f();",
        false,
    );
    check_case(
        "for_tuple",
        "let x=0; for (x,y) in [(1,2)] {while false invariant x==0 {}}",
        false,
    );
    check_case(
        "block",
        "let x=0; if true {let (x,y)=(1,2); while false invariant x==0 {}}",
        false,
    );
}

#[test]
fn noninteger_shadow_cannot_reuse_integer_domain() {
    check_case(
        "float",
        "let x=0; let (x,y)=(1.5,2); while false invariant x==x {}",
        false,
    );
    check_case(
        "bool",
        "let x=0; let (x,y)=(true,2); while false invariant x==x {}",
        false,
    );
}

#[test]
fn explicit_integer_rebinding_restores_proof_information() {
    check_case(
        "rebind",
        "let x=0; let (x,y)=(1,2); let x=3; while false invariant x==3 {}",
        true,
    );
}

#[test]
fn unrelated_binding_is_not_discarded() {
    check_case(
        "unrelated",
        "let x=0; let (a,b)=(1,2); while false invariant x==0 {}",
        true,
    );
    check_case(
        "field_name",
        "struct Pair { int x, } let x=0; let Pair {x:y}=new Pair {x:1}; while false invariant x==0 {}",
        true,
    );
}
