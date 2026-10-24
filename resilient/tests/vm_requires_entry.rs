//! RES-4976: actual VM calls check preconditions before body effects.
use std::{
    fs,
    process::{Command, Output},
};

fn run(case: &str, source: &str, mode: &str, dispatch: &str, optimize: bool) -> Output {
    let root = std::env::temp_dir().join(format!(
        "vm_requires_{}_{}_{}_{}",
        std::process::id(),
        case,
        mode,
        dispatch
    ));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("input.rz");
    fs::write(&path, source).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
    command
        .args(["--no-cache", "--audit", "--vm"])
        .arg(&path)
        .env("RESILIENT_DISPATCH", dispatch)
        .env_remove("RESILIENT_RICH_DIAG")
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .env_remove("RESILIENT_CONST_FOLD")
        .env_remove("RESILIENT_INLINE");
    if mode != "default" {
        command.env("RESILIENT_OVERFLOW_MODE", mode);
    }
    if optimize {
        command
            .env("RESILIENT_CONST_FOLD", "1")
            .env("RESILIENT_INLINE", "1");
    }
    let output = command.output().unwrap();
    if let Some(records) = std::env::var_os("VM_REQUIRES_RECORDS") {
        let destination =
            std::path::PathBuf::from(records).join(format!("{case}-{mode}-{dispatch}"));
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("input.rz"), source).unwrap();
        fs::write(destination.join("stdout.txt"), &output.stdout).unwrap();
        fs::write(destination.join("stderr.txt"), &output.stderr).unwrap();
        fs::write(
            destination.join("environment.txt"),
            format!("overflow={mode}\ndispatch={dispatch}\noptimize={optimize}\n"),
        )
        .unwrap();
        fs::write(
            destination.join("exit-status.txt"),
            output.status.to_string(),
        )
        .unwrap();
    }
    fs::remove_dir_all(root).unwrap();
    output
}
fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
fn violation(case: &str, source: &str, name: &str, clause: &str, optimize: bool) {
    for dispatch in ["match", "direct"] {
        let out = run(case, source, "default", dispatch, optimize);
        let diagnostic = text(&out);
        assert!(!out.status.success(), "{case}/{dispatch}: {diagnostic}");
        assert!(
            diagnostic.contains(&format!("Contract violation in fn {name}: requires"))
                && diagnostic.contains(clause),
            "{case}/{dispatch}: {diagnostic}"
        );
        assert!(
            !String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|line| line == "BODY_RAN"),
            "body ran: {diagnostic}"
        );
    }
}
#[test]
fn wrap_boundary_rejects_lia_tautology() {
    violation(
        "wrap_boundary",
        "fn f(int x) -> int requires x + 1 > x { return x; } println(f(9223372036854775807));",
        "f",
        "x + 1",
        false,
    );
}
#[test]
fn false_requires_precedes_body_side_effect() {
    violation(
        "body_effect",
        "fn f(int x) -> int requires x > 0 { println(\"BODY_RAN\"); return x; } println(f(0));",
        "f",
        "x > 0",
        false,
    );
}
#[test]
fn multiple_clauses_stop_before_later_fault() {
    violation("ordered_clauses", "fn f(int x) -> int requires x > 0 requires 1 / x > 0 { println(\"BODY_RAN\"); return x; } f(0);", "f", "x > 0", false);
}
#[test]
fn indirect_function_call_checks_actual_argument() {
    violation(
        "indirect",
        "fn f(int x) -> int requires x > 0 { return x; } let alias = f; println(alias(-1));",
        "f",
        "x > 0",
        false,
    );
}
#[test]
fn recursive_tail_entry_rechecks_rebound_argument() {
    violation("tail_recursive", "fn f(int n) -> int requires n >= 0 { if n == 0 { return f(-1); } return n; } println(f(0));", "f", "n >= 0", false);
}
#[test]
fn nested_requires_captures_outer_only_reference() {
    violation("nested_capture", "fn outer(int limit) -> int { fn inner(int x) -> int requires x > limit { return x; } return inner(1); } println(outer(5));", "inner", "x > limit", false);
}
#[test]
fn anonymous_requires_captures_outer_only_reference() {
    violation("anonymous_capture", "fn outer(int limit) -> int { let f = fn(int x) -> int requires x > limit { return x; }; return f(1); } println(outer(5));", "<anon>", "x > limit", false);
}
#[test]
fn module_function_preconditions_are_not_omitted() {
    violation(
        "module",
        "mod checks { fn f(int x) -> int requires x > 0 { return x; } } println(checks::f(0));",
        "checks::f",
        "x > 0",
        false,
    );
}
#[test]
fn optimization_cannot_remove_entry_checks() {
    violation("optimized", "fn f(int x) -> int requires x + 1 > x { println(\"BODY_RAN\"); return x; } println(f(9223372036854775807));", "f", "x + 1", true);
}
#[test]
fn valid_indirect_and_recursive_calls_remain_valid() {
    let source = "fn f(int x) -> int requires x > 0 { return x; } fn sum(int n) -> int requires n >= 0 { if n == 0 { return 0; } return n + sum(n - 1); } let alias = f; println(alias(7)); println(sum(4));";
    for dispatch in ["match", "direct"] {
        let out = run("valid_calls", source, "default", dispatch, false);
        assert!(out.status.success(), "{}", text(&out));
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.lines().any(|line| line == "7") && stdout.lines().any(|line| line == "10"),
            "{}",
            text(&out)
        );
    }
}
#[test]
fn arguments_and_requires_execute_once_in_source_order() {
    let source = "fn argument() -> int { println(\"ARG\"); return 7; } fn gate(int x) -> bool { println(\"CHECK\"); return x > 0; } fn f(int x) -> int requires gate(x) { println(\"BODY\"); return x; } println(f(argument()));";
    for dispatch in ["match", "direct"] {
        let out = run("evaluation_order", source, "default", dispatch, false);
        assert!(out.status.success(), "{}", text(&out));
        let stdout = String::from_utf8_lossy(&out.stdout);
        let observed: Vec<_> = stdout
            .lines()
            .filter(|line| ["ARG", "CHECK", "BODY", "7"].contains(line))
            .collect();
        assert_eq!(observed, ["ARG", "CHECK", "BODY", "7"], "{}", text(&out));
    }
}
#[test]
fn short_circuit_requires_does_not_evaluate_unreachable_rhs() {
    for dispatch in ["match", "direct"] {
        let out = run(
            "short_circuit",
            "fn f(int x) -> int requires x == 0 || 1 / x > 0 { return x; } println(f(0));",
            "default",
            dispatch,
            false,
        );
        assert!(out.status.success(), "{}", text(&out));
        assert!(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|line| line == "0"),
            "{}",
            text(&out)
        );
    }
}
#[test]
fn evaluated_requires_fault_preserves_underlying_error() {
    for dispatch in ["match", "direct"] {
        let out = run(
            "evaluated_fault",
            "fn f(int x) -> int requires 1 / x > 0 { println(\"BODY_RAN\"); return x; } f(0);",
            "default",
            dispatch,
            false,
        );
        assert!(!out.status.success(), "{}", text(&out));
        assert!(
            text(&out).to_lowercase().contains("division by zero"),
            "{}",
            text(&out)
        );
        assert!(
            !String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|line| line == "BODY_RAN"),
            "{}",
            text(&out)
        );
    }
}
#[test]
fn actual_vm_overflow_mode_controls_requires_evaluation() {
    let source =
        "fn f(int x) -> int requires x + 1 < 0 { return x; } println(f(9223372036854775807));";
    for dispatch in ["match", "direct"] {
        for mode in ["default", "saturate", "trap"] {
            let out = run("mode", source, mode, dispatch, false);
            let diagnostic = text(&out);
            if mode == "default" {
                assert!(out.status.success(), "{diagnostic}");
                assert!(
                    String::from_utf8_lossy(&out.stdout)
                        .lines()
                        .any(|line| line == "9223372036854775807"),
                    "{diagnostic}"
                );
            } else {
                assert!(!out.status.success(), "{diagnostic}");
                assert!(
                    if mode == "saturate" {
                        diagnostic.contains("Contract violation") && diagnostic.contains("requires")
                    } else {
                        diagnostic.to_lowercase().contains("overflow")
                    },
                    "{diagnostic}"
                );
            }
        }
    }
}

#[test]
fn valid_captured_and_module_entry_controls() {
    let cases = [
        ("valid_nested", "fn outer(int limit) -> int { fn inner(int x) -> int requires x > limit { return x; } return inner(7); } println(outer(5));"),
        ("valid_anonymous", "fn outer(int limit) -> int { let f = fn(int x) -> int requires x > limit { return x; }; return f(7); } println(outer(5));"),
        ("valid_module", "mod checks { fn f(int x) -> int requires x > 0 { return x; } } println(checks::f(7));"),
    ];
    for dispatch in ["match", "direct"] {
        for (case, source) in cases {
            let out = run(case, source, "default", dispatch, false);
            assert!(out.status.success(), "{case}/{dispatch}: {}", text(&out));
            assert!(
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .any(|line| line == "7"),
                "{}",
                text(&out)
            );
        }
    }
}
