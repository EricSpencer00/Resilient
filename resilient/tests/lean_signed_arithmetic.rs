use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn emit(expression: &str) -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "lean_signed_{}_{}.rz",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(
        &path,
        format!("fn sample(int a, int b) -> int {{ return {expression}; }}"),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "--emit-lean-spec=sample"])
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn division_output_binds_source_notation_to_truncating_arithmetic() {
    let theorem = emit("a / b");
    assert!(theorem.contains("local infixl:70 (priority := high) \" / \" => Int.tdiv"));
    assert!(theorem.contains("Expr.div"));
    assert!(theorem.contains("else none"));
}

#[test]
fn remainder_output_binds_source_notation_to_truncating_arithmetic() {
    let theorem = emit("a % b");
    assert!(theorem.contains("local infixl:70 (priority := high) \" % \" => Int.tmod"));
    assert!(theorem.contains("Expr.mod"));
    assert!(theorem.contains("else none"));
}

#[test]
fn nested_signed_arithmetic_keeps_both_local_notations() {
    let theorem = emit("(-a) / (b % 3)");
    assert_eq!(theorem.matches("local infixl").count(), 2);
    assert!(theorem.contains("lean_zero_"));
    assert!(!theorem.contains("sorry"));
}
