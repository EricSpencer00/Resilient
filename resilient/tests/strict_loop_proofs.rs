use std::{fs, process::Command};

fn run_case(name: &str, body: &str, provable: bool) {
    let root = std::env::temp_dir().join(format!("strict_loops_{}_{name}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("input.rz");
    fs::write(&path, format!("println(\"FORBIDDEN\");\n{body}")).unwrap();
    let mut modes = vec!["interpreter", "match", "direct"];
    if !provable {
        modes.push("jit");
    }
    for mode in modes {
        for flag_first in [true, false] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_rz"));
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("RESILIENT_") {
                    cmd.env_remove(key);
                }
            }
            if flag_first {
                cmd.arg("--deny-unproven-invariants");
            }
            cmd.arg(&path);
            if !flag_first {
                cmd.arg("--deny-unproven-invariants");
            }
            if mode == "jit" {
                cmd.arg("--jit");
            } else if mode != "interpreter" {
                cmd.arg("--vm").env("RESILIENT_DISPATCH", mode);
            }
            let output = cmd.output().unwrap();
            let success = provable && cfg!(feature = "z3");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(
                output.status.success(),
                success,
                "{name}/{mode}: {output:?}"
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).contains("FORBIDDEN"),
                success,
                "{output:?}"
            );
            if !success {
                assert!(
                    stderr.contains("unproven-loop-invariant"),
                    "{name}: {stderr}"
                );
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn proven_headers_and_statement_annotations() {
    run_case(
        "while",
        "let i=0; while i<2 invariant i>=0 && i<=2 {i=i+1;}",
        true,
    );
    run_case("for", "for i in [1,2] invariant i>=1 {}", true);
    run_case(
        "statement",
        "let i=0; while i<2 {invariant i>=0 && i<=2; i=i+1;}",
        true,
    );
}
#[test]
fn unsupported_and_false_annotations_stop_before_execution() {
    run_case(
        "unsupported",
        "let x=0; while x<1 invariant x>=0 {let ignored=0; x=x+1;}",
        false,
    );
    run_case("false", "while false invariant false {}", false);
    run_case("empty", "for i in [] invariant true {}", false);
    run_case(
        "float",
        "let x=sqrt(-1.0); while false invariant x==x {}",
        false,
    );
}
#[test]
fn successful_sites_do_not_mask_unproven_sites() {
    run_case(
        "closure",
        "let f=fn() {while false invariant true {}};",
        false,
    );
    run_case(
        "mixed",
        "while false invariant true {} while false invariant false {}",
        false,
    );
    run_case(
        "nested",
        "while false invariant true {while false invariant true {}}",
        false,
    );
    run_case(
        "function",
        "fn f() {while false invariant false {}} while false invariant true {}",
        false,
    );
}
#[test]
fn proof_budget_exhaustion_is_a_gate_failure() {
    run_case(
        "budget",
        &format!(
            "let x=0; while false invariant x==0 {{{}}}",
            "x=x+x;".repeat(24)
        ),
        false,
    );
}
#[test]
fn incompatible_flags_and_modes_cannot_skip_proofs() {
    let root = std::env::temp_dir().join(format!("strict_loop_options_{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("input.rz");
    fs::write(
        &path,
        "println(\"FORBIDDEN\"); while false invariant false {}",
    )
    .unwrap();
    for option in [
        "--no-typecheck",
        "--dump-tokens",
        "--dump-ast-json",
        "--dump-chunks",
        "--lsp",
        "--mcp",
        "--help",
        "--version",
        "--vibe-gate=1",
    ] {
        for reverse in [false, true] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_rz"));
            if reverse {
                cmd.args([option, "--deny-unproven-invariants"]);
            } else {
                cmd.args(["--deny-unproven-invariants", option]);
            }
            let output = cmd.arg(&path).output().unwrap();
            assert_eq!(output.status.code(), Some(2), "{option}: {output:?}");
            assert!(!String::from_utf8_lossy(&output.stdout).contains("FORBIDDEN"));
        }
    }
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn sources_without_annotations_need_no_solver() {
    let root = std::env::temp_dir().join(format!("strict_loop_empty_{}.rz", std::process::id()));
    fs::write(&root, "println(42);").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg(&root)
        .arg("--deny-unproven-invariants")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line == "42")
    );
    fs::remove_file(root).unwrap();
}
#[test]
fn library_gate_is_explicit_and_reports_source_sites() {
    let result = resilient::run_program_with_options(
        "println(\"FORBIDDEN\"); while false invariant false {}",
        resilient::RunOptions {
            deny_unproven_invariants: true,
        },
    );
    assert!(!result.ok);
    assert!(result.stdout.is_empty());
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("<input>:1:") && error.contains("unproven-loop-invariant")),
        "{:?}",
        result.errors
    );
}

#[test]
fn warm_default_cache_cannot_supply_strict_proof_credit() {
    let root = std::env::temp_dir().join(format!("strict_loop_cache_{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("input.rz");
    fs::write(
        &path,
        "println(\"FORBIDDEN\"); while false invariant true {let ignored=0;}",
    )
    .unwrap();
    for vm in [false, true] {
        let mut warm = Command::new(env!("CARGO_BIN_EXE_rz"));
        warm.arg(&path);
        if vm {
            warm.arg("--vm");
        }
        let output = warm.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let mut strict = Command::new(env!("CARGO_BIN_EXE_rz"));
        strict.arg(&path).arg("--deny-unproven-invariants");
        if vm {
            strict.arg("--vm");
        }
        let output = strict.output().unwrap();
        assert!(!output.status.success(), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("FORBIDDEN"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("unproven-loop-invariant"));
    }
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn imported_unproven_annotations_are_not_omitted() {
    let root = std::env::temp_dir().join(format!("strict_loop_import_{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("input.rz");
    fs::write(
        root.join("dep.rz"),
        "fn unused() {while false invariant false {}}",
    )
    .unwrap();
    fs::write(
        &path,
        "use \"dep.rz\"; println(\"FORBIDDEN\"); while false invariant true {}",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg(&path)
        .arg("--deny-unproven-invariants")
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("FORBIDDEN"));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unproven-loop-invariant"),
        "{output:?}"
    );
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn library_default_behavior_is_preserved() {
    let source = "println(42); while false invariant true {let ignored=0;}";
    let result = resilient::run_program_with_options(source, resilient::RunOptions::default());
    assert!(result.ok, "{:?}", result.errors);
    assert_eq!(result.stdout.trim(), "42");
}
