use std::{fs, process::Command};

fn check(name: &str, source: &str, expected_stdout: &str, success: bool) {
    let root = std::env::temp_dir().join(format!("loop_bound_{}_{name}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("input.rz");
    fs::write(&path, source).unwrap();
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
        command.arg(&path);
        if mode == "jit" {
            command.arg("--jit");
        } else if mode != "interpreter" {
            command.arg("--vm").env("RESILIENT_DISPATCH", mode);
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.success(),
            success,
            "{name}/{mode}: {output:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .trim_end_matches("Program executed successfully")
                .trim_end(),
            expected_stdout,
            "{name}/{mode}: {output:?}"
        );
        if !success {
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("loop bound exceeded"),
                "{name}/{mode}: {output:?}"
            );
        }
    }
    let output = resilient::run_program(source);
    assert_eq!(output.ok, success, "{name}/in-memory: {:?}", output.errors);
    assert_eq!(output.stdout.trim(), expected_stdout, "{name}/in-memory");
    if !success {
        assert!(output.errors.join("\n").contains("loop bound exceeded"));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stops_before_excess_body_side_effects() {
    check(
        "inclusive",
        "#[loop_bound(1)] fn f(){let i=0;while i<=1 {println(i);i=i+1;}} f();",
        "0",
        false,
    );
    check(
        "conditional",
        "#[loop_bound(2)] fn f(){let i=0;while i<4 {println(i);if false {i=i+2;} else {i=i+1;}}} f();",
        "0\n1",
        false,
    );
    check(
        "overwrite",
        "#[loop_bound(2)] fn f(){let i=0;while i<4 {println(i);i=i+2;i=i-1;}} f();",
        "0\n1",
        false,
    );
}

#[test]
fn exact_limit_and_repeat_calls_reset_budget() {
    check(
        "exact",
        "#[loop_bound(2)] fn f(){let i=0;while i<2 {println(i);i=i+1;}} f();f();",
        "0\n1\n0\n1",
        true,
    );
    check(
        "zero",
        "#[loop_bound(1)] fn f(){while false {println(99);}} f();",
        "",
        true,
    );
}

#[test]
fn continue_counts_and_break_at_limit_succeeds() {
    check(
        "continue",
        "#[loop_bound(2)] fn f(){let i=0;while i<3 {i=i+1;println(i);continue;}} f();",
        "1\n2",
        false,
    );
    check(
        "break",
        "#[loop_bound(2)] fn f(){let i=0;while true {i=i+1;println(i);if i==2 {break;}}} f();",
        "1\n2",
        true,
    );
}

#[test]
fn nested_loop_budget_resets_per_activation() {
    check(
        "nested",
        "#[loop_bound(2)] fn f(){let i=0;while i<2 {let j=0;while j<2 {println(j);j=j+1;}i=i+1;}} f();",
        "0\n1\n0\n1",
        true,
    );
    check(
        "nested_excess",
        "#[loop_bound(2)] fn f(){let i=0;while i<2 {let j=0;while j<3 {println(j);j=j+1;}i=i+1;}} f();",
        "0\n1",
        false,
    );
}

#[test]
fn unannotated_loops_keep_existing_behavior() {
    check(
        "unannotated",
        "fn f(){let i=0;while i<3 {println(i);i=i+1;}} f();",
        "0\n1\n2",
        true,
    );
}

#[test]
fn structured_paths_cannot_hide_bounded_loops() {
    check(
        "match",
        "#[loop_bound(1)] fn f(int n){return match n {0 => {let i=0;while i<2 {println(i);i=i+1;} 0}, _ => 0};} f(0);",
        "0",
        false,
    );
    check(
        "try",
        "#[loop_bound(1)] fn f(){try {let i=0;while i<2 {println(i);i=i+1;}} catch Timeout {println(99);}} f();",
        "0",
        false,
    );
}

#[test]
fn static_estimates_distinguish_inclusive_and_unqualified_progress() {
    let root = std::env::temp_dir().join(format!("loop_static_{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for (name, bound, condition, step, rejected, warning) in [
        ("exclusive", 1, "i<1", "i=i+1;", false, false),
        ("inclusive", 1, "i<=1", "i=i+1;", true, false),
        (
            "conditional",
            2,
            "i<4",
            "if false {i=i+2;} else {i=i+1;}",
            false,
            true,
        ),
        ("overwrite", 2, "i<4", "i=i+2;i=i-1;", false, true),
        (
            "huge_step",
            4294967295_u32,
            "i<1",
            "i=i+4294967295;",
            false,
            false,
        ),
        (
            "truncate_step",
            1,
            "i<4294967297",
            "i=i+4294967296;",
            true,
            false,
        ),
    ] {
        let path = root.join(format!("{name}.rz"));
        fs::write(&path, format!("@require_contracts(strict)\n#[loop_bound({bound})]\nfn f(int n)->int requires n>=0 ensures result>=0 {{let i=0;while {condition} {{{step}}} return i;}}" )).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_rz"))
            .args(["--typecheck", "--verbose"])
            .arg(&path)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            !output.status.success(),
            rejected && cfg!(feature = "z3"),
            "{name}: {output:?}"
        );
        if warning && cfg!(feature = "z3") {
            assert!(stderr.contains("warning[loop_bound]"), "{name}: {stderr}");
        }
        if rejected && cfg!(feature = "z3") {
            assert!(
                stderr.contains("may exceed its declared bound"),
                "{name}: {stderr}"
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn in_memory_invocations_do_not_share_loop_budgets() {
    let handles: Vec<_> = (1..=4).map(|bound| std::thread::spawn(move || {
        for _ in 0..10 {
            let source = format!("#[loop_bound({bound})] fn shared(){{let i=0;while i<5 {{println(i);i=i+1;}}}} shared();");
            let output = resilient::run_program(&source);
            assert!(!output.ok, "{:?}", output.errors);
            assert_eq!(output.stdout.lines().count(), bound);
            assert!(output.errors.join("\n").contains("loop bound exceeded"));
            let unbounded = resilient::run_program("fn shared(){let i=0;while i<5 {println(i);i=i+1;}} shared();");
            assert!(unbounded.ok, "{:?}", unbounded.errors);
            assert_eq!(unbounded.stdout.lines().count(), 5);
        }
    })).collect();
    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn generic_specialization_preserves_budget() {
    check(
        "generic",
        "#[loop_bound(1)] fn f<T>(T x)->T {let i=0;while i<2 {println(i);i=i+1;}return x;} f(7);",
        "0",
        false,
    );
}

#[test]
fn generated_guards_do_not_replace_source_invariant_proofs() {
    let path =
        std::env::temp_dir().join(format!("loop_bound_source_proof_{}.rz", std::process::id()));
    fs::write(
        &path,
        "#[loop_bound(2)] fn f(){let i=0;while i<2 invariant i>=0 {i=i+1;}} f();",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--deny-unproven-invariants", "--verbose", "--vm"])
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(output.status.success(), cfg!(feature = "z3"), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(if cfg!(feature = "z3") {
            "invariant proven"
        } else {
            "unproven-loop-invariant:"
        }),
        "{stderr}"
    );
    fs::remove_file(path).unwrap();
}
