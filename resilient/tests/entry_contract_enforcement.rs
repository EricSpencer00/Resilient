//! Preconditions must guard every actual function entry, including optimized calls.
use sha2::Digest;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
fn exercise(case: &str, source: &str, violation: bool, numbers: &[&str]) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "entry_contract_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("input.rz"), source).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--audit", "--no-cache"])
        .arg(root.join("input.rz"))
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut timed_out = false;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
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
    static BINARY_SHA: OnceLock<String> = OnceLock::new();
    static RUSTC: OnceLock<String> = OnceLock::new();
    let binary_sha = BINARY_SHA.get_or_init(|| {
        format!(
            "{:x}",
            sha2::Sha256::digest(std::fs::read(env!("CARGO_BIN_EXE_rz")).unwrap())
        )
    });
    let rustc = RUSTC.get_or_init(|| {
        String::from_utf8(
            Command::new("rustc")
                .arg("--version")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned()
    });
    let actual_numbers: Vec<_> = stdout
        .lines()
        .filter(|line| line.parse::<i64>().is_ok())
        .collect();
    let satisfied = !timed_out
        && if violation {
            !output.status.success()
                && diagnostic.contains("Contract violation")
                && diagnostic.contains("requires")
                && !stdout.contains("FORBIDDEN_BODY")
        } else {
            output.status.success() && actual_numbers == numbers
        };
    let record = serde_json::json!({"schema":"resilient-entry-cli-record/v2", "case":case,"source":source,"source_sha256":format!("{:x}",sha2::Sha256::digest(source.as_bytes())),"argv":["--audit","--no-cache",root.join("input.rz")],"overflow_environment":"unset","effective_overflow_mode":"Wrap","features":{"z3":cfg!(feature="z3"),"jit":cfg!(feature="jit"),"lsp":cfg!(feature="lsp")},"compiler_path":env!("CARGO_BIN_EXE_rz"),"compiler_sha256":binary_sha,"rustc":rustc,"exit_code":output.status.code(),"timed_out":timed_out,"stdout":stdout,"stderr":String::from_utf8_lossy(&output.stderr),"expected_requires_violation":violation,"expected_numeric_output":numbers,"requirement_satisfied":satisfied});
    let line = format!("RESILIENT_ENTRY_RECORD {}\n", record);
    std::io::stdout().lock().write_all(line.as_bytes()).unwrap();
    assert!(!timed_out, "CLI child timed out: {output:?}");
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
