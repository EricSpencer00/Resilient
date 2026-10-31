use std::{fs, process::Command};

fn check_case(name: &str, source: &str, provable: bool) {
    check_policy_case(name, source, provable, "wrap");
}

fn check_policy_case(name: &str, source: &str, provable: bool, policy: &str) {
    let root = std::env::temp_dir().join(format!("loop_constants_{}_{name}", std::process::id()));
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
            .env("RESILIENT_OVERFLOW_MODE", policy)
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
fn later_and_shadowed_constants_cannot_supply_false_proofs() {
    check_case(
        "late_false",
        "let x=0; while false invariant x==0 {} const x=1;",
        false,
    );
    check_case(
        "shadow_false",
        "const x=1; let x=0; while false invariant x==0 {}",
        false,
    );
    check_case(
        "function_false",
        "const x=1; fn f(){let x=0; while false invariant x==0 {}} f();",
        false,
    );
}

#[test]
fn runtime_constant_values_supply_valid_proofs() {
    check_case(
        "late_true",
        "let x=0; while false invariant x==1 {} const x=1;",
        true,
    );
    check_case(
        "shadow_true",
        "const x=1; let x=0; while false invariant x==1 {}",
        true,
    );
    check_case(
        "expression",
        "const BASE=2; const x=BASE+3; let x=0; while false invariant x==5 {}",
        true,
    );
    check_case(
        "parameter",
        "const x=1; fn f(int x){while false invariant x==1 {}} f(0);",
        true,
    );
}

#[test]
fn finite_loop_binding_does_not_override_a_constant() {
    check_case(
        "for_false",
        "const x=1; for x in [0,2] invariant x==0 {}",
        false,
    );
    check_case(
        "for_true",
        "const x=1; for x in [0,2] invariant x==1 {}",
        true,
    );
}

#[test]
fn noninteger_constant_cannot_supply_an_integer_domain() {
    check_case(
        "nan_direct",
        "const x=0.0/0.0; let x=0; while false invariant x==x {}",
        false,
    );
    check_case(
        "nan_alias",
        "const x=0.0/0.0; let x=0; let y=x; while false invariant y==y {}",
        false,
    );
    check_case(
        "nan_for",
        "const x=0.0/0.0; for x in [0] invariant x==x {}",
        false,
    );
}

#[test]
fn nonhoisted_function_const_is_not_an_initialized_runtime_local() {
    check_case(
        "local_const",
        "fn f(){const x=0; while false invariant x==0 {}} f();",
        false,
    );
}

#[test]
fn ordinary_mutable_loop_bindings_still_prove() {
    check_case(
        "mutable",
        "const LIMIT=2; let i=0; while i<LIMIT invariant i>=0 && i<=LIMIT {i=i+1;}",
        true,
    );
    check_case(
        "unrelated",
        "const OTHER=3; for x in [0,1] invariant x>=0 {}",
        true,
    );
}

#[test]
fn bounded_scalar_constants_and_unrelated_strings_keep_valid_proofs() {
    check_case(
        "conditional",
        "const x=if 1<2 {3} else {4}; let x=0; while false invariant x==3 {}",
        true,
    );
    check_case(
        "unrelated_string",
        "const MESSAGE=\"ok\"; let x=0; while false invariant x==0 {}",
        true,
    );
    check_case(
        "string_shadow",
        "const x=\"ok\"; let x=0; while false invariant x==0 {}",
        false,
    );
}

#[test]
fn resolved_constants_use_the_selected_overflow_policy() {
    check_policy_case(
        "wrap",
        "const x=9223372036854775807+1; while false invariant x<0 {}",
        true,
        "wrap",
    );
    check_policy_case(
        "saturate",
        "const x=9223372036854775807+1; while false invariant x==9223372036854775807 {}",
        true,
        "saturate",
    );
    check_policy_case(
        "trap",
        "const x=9223372036854775807+1; while false invariant x==x {}",
        false,
        "trap",
    );
    check_policy_case(
        "wrong_wrap",
        "const x=9223372036854775807+1; while false invariant x==9223372036854775807 {}",
        false,
        "wrap",
    );
    check_policy_case(
        "wrong_saturate",
        "const x=9223372036854775807+1; while false invariant x<0 {}",
        false,
        "saturate",
    );
}
