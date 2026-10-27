//! Terminal members must retain tail-call entry contracts without a return edge.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "terminal-tail-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn run(&self, source: &str) -> std::process::Output {
        let input = self.0.join("input.rz");
        std::fs::write(&input, source).unwrap();
        Command::new(env!("CARGO_BIN_EXE_rz"))
            .args(["--typecheck", "--no-cache"])
            .arg(input)
            .output()
            .expect("run compiler")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn terminal_member_returns_without_a_cross_call_back() {
    let output = Fixture::new().run(
        "#[mutual_tail_call] fn first(int n) -> int { return terminal(n); } #[mutual_tail_call] fn terminal(int n) -> int { return n + 1; } println(first(41));",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line == "42")
    );
}

#[test]
fn explicit_tail_return_before_a_following_statement_counts() {
    let output = Fixture::new().run(
        "#[mutual_tail_call] fn first(int n) -> int { if n > 0 { return terminal(n); } return 0; } #[mutual_tail_call] fn terminal(int n) -> int { return n + 1; } println(first(41));",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line == "42")
    );
}

#[test]
fn terminal_target_precondition_runs_before_its_body() {
    let output = Fixture::new().run(
        "fn opaque(int n) -> int { return n; } #[mutual_tail_call] fn first(int n) -> int requires n >= 0 { return terminal(opaque(n - 1)); } #[mutual_tail_call] fn terminal(int n) -> int requires n > 0 { println(\"FORBIDDEN_BODY\"); return n; } println(first(1));",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("Type check passed"), "{stdout}\n{stderr}");
    assert!(!output.status.success());
    assert!(!stdout.contains("FORBIDDEN_BODY"));
    assert!(
        stderr.contains("Contract violation in fn terminal: requires"),
        "{stderr}"
    );
    assert!(stderr.contains("n > 0"), "{stderr}");
}

#[test]
fn unannotated_partner_does_not_form_a_group() {
    let output = Fixture::new().run(
        "#[mutual_tail_call] fn first(int n) -> int { return terminal(n); } fn terminal(int n) -> int { return n; }",
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mutual_tail_call"));
}

#[test]
fn calls_from_unannotated_functions_do_not_validate_lone_annotations() {
    let output = Fixture::new().run(
        "fn plain(int n) -> int { return terminal(n); } #[mutual_tail_call] fn terminal(int n) -> int { return n; }",
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mutual_tail_call"));
}

#[test]
fn non_tail_argument_calls_do_not_validate_terminal_members() {
    let output = Fixture::new().run(
        "fn passthrough(int n) -> int { return n; } #[mutual_tail_call] fn first(int n) -> int { return passthrough(terminal(n)); } #[mutual_tail_call] fn terminal(int n) -> int { return n; }",
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("function `terminal`"));
}
