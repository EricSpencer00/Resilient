//! Preconditions must guard every actual function entry, including optimized calls.
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
fn exercise(case: &str, source: &str, violation: bool, numbers: &[&str]) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "entry_contract_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("input.rz"), source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--audit", "--no-cache"])
        .arg(root.join("input.rz"))
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .output()
        .unwrap();
    std::fs::write(root.join("stdout.txt"), &output.stdout).unwrap();
    std::fs::write(root.join("stderr.txt"), &output.stderr).unwrap();
    std::fs::write(root.join("exit-status.txt"), output.status.to_string()).unwrap();
    if let Some(destination) = std::env::var_os("ENTRY_CONTRACT_RECORDS") {
        let destination = std::path::PathBuf::from(destination).join(case);
        std::fs::create_dir_all(&destination).unwrap();
        for name in ["input.rz", "stdout.txt", "stderr.txt", "exit-status.txt"] {
            std::fs::copy(root.join(name), destination.join(name)).unwrap();
        }
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let diagnostic = format!("{stdout}{}", String::from_utf8_lossy(&output.stderr));
    if violation {
        assert!(!output.status.success(), "{output:?}");
        assert!(
            diagnostic.contains("Contract violation") && diagnostic.contains("requires"),
            "{diagnostic}"
        );
        assert!(
            !stdout.contains("FORBIDDEN_BODY"),
            "false-entry body ran: {stdout}"
        );
    } else {
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            stdout
                .lines()
                .filter(|line| line.parse::<i64>().is_ok())
                .collect::<Vec<_>>(),
            numbers,
            "{stdout}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn self_tail_entry_rechecks_requires_before_body() {
    exercise(
        "self_tail_entry_rechecks_requires_before_body",
        "#[must_tail_call] fn positive(int n) -> int requires n > 0 { if n == 1 { return positive(n - 1); } println(\"FORBIDDEN_BODY\"); return n; } println(positive(1));",
        true,
        &[],
    );
}
#[test]
fn valid_self_tail_entries_preserve_results() {
    exercise(
        "valid_self_tail_entries_preserve_results",
        "#[must_tail_call] fn countdown(int n) -> int requires n >= 0 { if n == 0 { return n; } return countdown(n - 1); } println(countdown(2));",
        false,
        &["0"],
    );
}
#[test]
fn mutual_tail_entry_checks_target_requires_before_body() {
    exercise(
        "mutual_tail_entry_checks_target_requires_before_body",
        "#[mutual_tail_call] fn first(int n) -> int requires n > 0 { if n == 1 { return second(n - 1); } return n; } #[mutual_tail_call] fn second(int n) -> int requires n > 0 { if n == 0 { println(\"FORBIDDEN_BODY\"); return n; } return first(n - 1); } println(first(1));",
        true,
        &[],
    );
}
#[test]
fn valid_mutual_tail_entries_preserve_results() {
    exercise(
        "valid_mutual_tail_entries_preserve_results",
        "#[mutual_tail_call] fn first(int n) -> int requires n >= 0 { if n == 0 { return n; } return second(n - 1); } #[mutual_tail_call] fn second(int n) -> int requires n >= 0 { if n == 0 { return n; } return first(n - 1); } println(first(2));",
        false,
        &["0"],
    );
}
#[test]
fn alias_keeps_guard_after_direct_call_proof() {
    exercise(
        "alias_keeps_guard_after_direct_call_proof",
        "fn positive(int x) -> int requires x > 0 { if x < 0 { println(\"FORBIDDEN_BODY\"); } return x; } positive(5); let alias = positive; println(alias(-1));",
        true,
        &[],
    );
}
#[test]
fn valid_aliased_call_preserves_result() {
    exercise(
        "valid_aliased_call_preserves_result",
        "fn positive(int x) -> int requires x > 0 { return x; } positive(5); let alias = positive; println(alias(7));",
        false,
        &["7"],
    );
}
#[test]
fn ordinary_recursive_entry_keeps_requires() {
    exercise(
        "ordinary_recursive_entry_keeps_requires",
        "fn positive(int n) -> int requires n > 0 { if n == 1 { return positive(n - 1); } println(\"FORBIDDEN_BODY\"); return n; } println(positive(1));",
        true,
        &[],
    );
}
