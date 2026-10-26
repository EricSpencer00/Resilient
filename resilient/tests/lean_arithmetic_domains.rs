use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn emit(expression: &str) -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let source = std::env::temp_dir().join(format!(
        "lean_domains_{}_{}.rz",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(
        &source,
        format!("fn sample(int a, int b, int c) -> int {{ return {expression}; }}"),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "--emit-lean-spec=sample"])
        .arg(&source)
        .output()
        .unwrap();
    std::fs::remove_file(source).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn division_and_remainder_describe_both_defined_and_undefined_results() {
    for expression in ["a / b", "a % b", "a / 0", "a % 0"] {
        let theorem = emit(expression);
        assert!(theorem.contains("≠ 0"), "missing domain: {theorem}");
        assert!(
            theorem.contains("else none"),
            "missing undefined result: {theorem}"
        );
        assert!(theorem.contains("by_cases lean_zero_0"));
        assert!(!theorem.contains("sorry"));
    }
}

#[test]
fn nested_partial_operands_contribute_their_own_domains() {
    let theorem = emit("a / (b % c)");
    assert!(theorem.contains("lean_arg_2 ≠ 0"));
    assert!(theorem.contains("(lean_arg_1 % lean_arg_2) ≠ 0"));
    assert!(theorem.contains(" ∧ "));
    let propagated = emit("(a / b) + (a % c)");
    assert!(propagated.contains("lean_arg_1 ≠ 0"));
    assert!(propagated.contains("lean_arg_2 ≠ 0"));
    assert!(propagated.contains("else none"));
}

#[test]
fn total_arithmetic_keeps_an_unconditional_some_result() {
    let theorem = emit("a * (b + c)");
    assert!(!theorem.contains("else none"));
    assert!(!theorem.contains("lean_zero_"));
}
