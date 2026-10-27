use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn emit(source: &str) -> std::process::Output {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "lean_typed_{}_{}.rz",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "--emit-lean-spec=sample"])
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    result
}

#[test]
fn boolean_results_and_parameters_use_integer_encoding() {
    for source in [
        "fn sample() -> bool { return true; }",
        "fn sample(bool flag) -> bool { return flag; }",
        "fn sample(bool flag) -> bool { return !flag; }",
        "fn sample(bool a, bool b) -> bool { return a == b; }",
        "fn sample(int a, int b) -> bool { return a < b; }",
    ] {
        let result = emit(source);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let theorem = String::from_utf8(result.stdout).unwrap();
        assert!(theorem.contains("Resilient.eval"));
        assert!(
            theorem.contains("Int"),
            "missing integer encoding: {theorem}"
        );
        assert!(!theorem.contains("some true"));
        assert!(!theorem.contains("some false"));
    }
}

#[test]
fn invalid_or_unsupported_signatures_and_operands_are_rejected() {
    for source in [
        "fn sample(string x) -> int { return 1; }",
        "fn sample(int x) -> string { return x; }",
        "fn sample() -> bool { return 1; }",
        "fn sample() -> int { return true; }",
        "fn sample(bool x) -> int { return x + 1; }",
        "fn sample(int x) -> bool { return !x; }",
        "fn sample(bool x) -> int { return -x; }",
        "fn sample(int x, bool y) -> bool { return x == y; }",
        "fn sample(bool x, bool y) -> bool { return x < y; }",
        "fn sample() -> int { return unknown; }",
        "fn sample(int x, int x) -> int { return x; }",
    ] {
        let result = emit(source);
        assert!(
            !result.status.success(),
            "unexpected theorem for {source}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(String::from_utf8_lossy(&result.stderr).contains("cannot lower"));
    }
}

#[test]
fn parameter_names_cannot_shadow_the_emitted_expression_reference() {
    let result = emit("fn sample(int sample, bool eval) -> int { return sample; }");
    assert!(result.status.success());
    let theorem = String::from_utf8(result.stdout).unwrap();
    assert!(theorem.contains("_root_.sample"));
    assert!(theorem.contains("lean_arg_0 : Int"));
    assert!(theorem.contains("lean_arg_1 : Bool"));
}
