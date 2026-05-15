//! RES-198: integration tests for `resilient lint <file>`.
//!
//! Each test writes a tiny program to a temp file, shells out to
//! the real binary, and asserts on the exit code + stdout/stderr.
//! The unit tests in `src/lint.rs` cover the detection logic;
//! these pin the CLI wiring: arg parsing, exit codes, `--deny` /
//! `--allow` escalation, unknown-code handling.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_file(tag: &str, body: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("res_198_{}_{}_{}.rs", tag, std::process::id(), n));
    std::fs::write(&path, body).expect("write scratch");
    path
}

#[test]
fn lint_exits_zero_on_clean_program() {
    // RES-397: `// source:` comment satisfies L0012 spec-provenance lint.
    // Call `f` at top level so L0014 (unused function) does not fire.
    let src = tmp_file(
        "clean",
        "// source: test fixture\nfn f(int a) requires a > 0 {\n    let used = a + 1;\n    return used;\n}\nf(1);\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    assert!(
        out.status.success(),
        "expected 0, got {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_exits_one_on_warning() {
    let src = tmp_file(
        "warn",
        "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected 1 (warning), got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("L0001"), "stdout: {stdout}");
    assert!(stdout.contains("warning"), "stdout: {stdout}");
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_deny_escalates_to_error_exit_two() {
    let src = tmp_file(
        "deny",
        "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .args(["--deny", "L0001"])
        .output()
        .expect("spawn lint");
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected 2 (error), got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("error[L0001]"), "stdout: {stdout}");
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_allow_flag_suppresses_code() {
    // RES-397: `// source:` comment satisfies L0012; `--allow L0001`
    // silences the unused-let warning, leaving a clean lint.
    // Call `f` so L0014 does not fire alongside L0001.
    let src = tmp_file(
        "allow",
        "// source: test fixture\nfn f(int a) requires a > 0 {\n    let unused = 42;\n    return a;\n}\nf(1);\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .args(["--allow", "L0001"])
        .output()
        .expect("spawn lint");
    assert!(
        out.status.success(),
        "expected 0 under --allow L0001, got {:?}\nstdout: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_rejects_unknown_deny_code() {
    let src = tmp_file("unknown_deny", "fn f() { return 0; }\n");
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .args(["--deny", "LX999"])
        .output()
        .expect("spawn lint");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown lint code"), "stderr: {stderr}");
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_requires_file_argument() {
    let out = Command::new(bin())
        .args(["lint"])
        .output()
        .expect("spawn lint");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("<file>") || stderr.contains("file path"),
        "stderr: {stderr}"
    );
}

#[test]
fn lint_errors_on_missing_file() {
    let out = Command::new(bin())
        .args(["lint", "/nonexistent/path.rs"])
        .output()
        .expect("spawn lint");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn lint_prints_location_in_rust_like_format() {
    let src = tmp_file(
        "fmt",
        "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let src_path = src.to_string_lossy().to_string();
    // Expected: `<path>:<line>:<col>: warning[L0001]: ...`
    let prefix = format!("{}:2:", src_path);
    assert!(
        stdout.contains(&prefix),
        "expected `{prefix}...` in stdout, got: {stdout}"
    );
    let _ = std::fs::remove_file(&src);
}

// ---------- RES-308: L0011 unused variable warning ----------

#[test]
fn lint_l0011_fires_on_unused_let_with_rustc_message() {
    // RES-308 acceptance: an unused `let` binding emits L0011
    // with the rustc-style phrasing.
    let src = tmp_file(
        "l0011_warn",
        "fn f(int a) requires a > 0 {\n    let unused = 42;\n    return a;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("warning[L0011]"),
        "expected warning[L0011] in stdout, got: {stdout}"
    );
    assert!(
        stdout.contains("variable `unused` is assigned but never used"),
        "expected rustc-style message in stdout, got: {stdout}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 for warning, got {:?}",
        out.status
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_l0011_silent_for_underscore_prefix() {
    // `_temp` is exempt — file is clean, exit 0.
    // RES-397: `// source:` satisfies L0012. Call `f` to silence L0014.
    let src = tmp_file(
        "l0011_underscore",
        "// source: test fixture\nfn f(int a) requires a > 0 {\n    let _temp = 42;\n    return a;\n}\nf(1);\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    assert!(
        out.status.success(),
        "expected 0 for `_`-prefixed binding, got {:?}\nstdout: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_l0011_silent_when_let_is_used() {
    // Used `let` is clean. RES-397: `// source:` satisfies L0012.
    // Call `f` to silence L0014 (unused function).
    let src = tmp_file(
        "l0011_used",
        "// source: test fixture\nfn f(int a) requires a > 0 {\n    let used = a + 1;\n    return used;\n}\nf(1);\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    assert!(
        out.status.success(),
        "expected 0 for used binding, got {:?}\nstdout: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_l0011_deny_escalates_to_error_exit_two() {
    // RES-308 acceptance: `--deny L0011` escalates to error.
    let src = tmp_file(
        "l0011_deny",
        "fn f(int a) requires a > 0 {\n    let unused = 42;\n    return a;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .args(["--deny", "L0011"])
        .output()
        .expect("spawn lint");
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected 2 (error), got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("error[L0011]"),
        "expected error[L0011] under --deny, got: {stdout}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_handles_multiple_codes_per_invocation() {
    // A program triggering L0001 + L0003 + L0005. --deny L0003
    // should fail with error exit 2; L0001 + L0005 stay as
    // warnings (so even without their own --deny, the overall
    // exit is driven by the error).
    let src = tmp_file(
        "multi",
        "fn f(int x) {\n    let unused = 42;\n    if x == x { return 1; }\n    return;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .args(["--deny", "L0003"])
        .output()
        .expect("spawn lint");
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // L0003 escalated to error.
    assert!(stdout.contains("error[L0003]"), "stdout: {stdout}");
    // L0001 + L0005 still warnings.
    assert!(stdout.contains("warning[L0001]"), "stdout: {stdout}");
    assert!(stdout.contains("warning[L0005]"), "stdout: {stdout}");
    let _ = std::fs::remove_file(&src);
}

// ---------- L0015: constant expression integer overflow ----------

#[test]
fn lint_l0015_fires_on_overflow() {
    // i64::MAX + 1 overflows.
    let src = tmp_file(
        "l0015_overflow",
        "fn f() -> int { return 9223372036854775807 + 1; }\nf();\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("warning[L0015]"),
        "expected warning[L0015] in stdout, got: {stdout}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 for L0015 warning, got {:?}",
        out.status
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_l0015_silent_for_valid_arithmetic() {
    // 100 + 200 does not overflow — should exit 0.
    // RES-397: `// source:` satisfies L0012. Call `f` to silence L0014.
    let src = tmp_file(
        "l0015_clean",
        "// source: test fixture\nfn f(int x) -> int requires x >= 0 { return 100 + 200; }\nf(1);\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    assert!(
        out.status.success(),
        "expected exit 0 for non-overflowing arithmetic, got {:?}\nstdout: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
    );
    let _ = std::fs::remove_file(&src);
}

// ---------- L0016: constant boolean condition ----------

#[test]
fn lint_l0016_fires_on_literal_true() {
    let src = tmp_file("l0016_true", "fn f() { if true { let _x = 1; } }\nf();\n");
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("warning[L0016]"),
        "expected warning[L0016] in stdout, got: {stdout}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_l0016_silent_for_variable_condition() {
    // RES-397: `// source:` satisfies L0012. Call `f` to silence L0014.
    let src = tmp_file(
        "l0016_var",
        "// source: test fixture\nfn f(int x) -> int requires x > 0 { if x > 0 { return 1; } return 0; }\nf(1);\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    assert!(
        out.status.success(),
        "expected exit 0 for variable condition, got {:?}\nstdout: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
    );
    let _ = std::fs::remove_file(&src);
}

// ---------- --emit-diagnostics-json flag ----------

#[test]
fn lint_emit_diagnostics_json_produces_valid_json_array() {
    let src = tmp_file(
        "json_output",
        "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .arg("--emit-diagnostics-json")
        .output()
        .expect("spawn lint --emit-diagnostics-json");
    // Should exit 1 (warning) or 0; not 2 (error) for basic warnings.
    assert_ne!(
        out.status.code(),
        Some(2),
        "expected non-error exit, got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Output must be a valid JSON array.
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "expected valid JSON array, got: {stdout}");
    let arr = parsed.unwrap();
    assert!(
        arr.is_array(),
        "expected JSON array at top level, got: {stdout}"
    );
    // Each element must have severity, code, line, column, message, file fields.
    for diag in arr.as_array().unwrap() {
        assert!(
            diag.get("severity").is_some(),
            "missing severity field: {diag}"
        );
        assert!(diag.get("code").is_some(), "missing code field: {diag}");
        assert!(diag.get("line").is_some(), "missing line field: {diag}");
        assert!(diag.get("column").is_some(), "missing column field: {diag}");
        assert!(
            diag.get("message").is_some(),
            "missing message field: {diag}"
        );
        assert!(diag.get("file").is_some(), "missing file field: {diag}");
    }
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_emit_diagnostics_json_empty_for_clean_file() {
    // Clean file with source: comment, requires, and top-level call.
    let src = tmp_file(
        "json_clean",
        "// source: test fixture\nfn f(int a) requires a > 0 { let used = a + 1; return used; }\nf(1);\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .arg("--emit-diagnostics-json")
        .output()
        .expect("spawn lint --emit-diagnostics-json clean");
    assert!(
        out.status.success(),
        "expected exit 0 for clean file with JSON flag, got {:?}\nstdout: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let arr: serde_json::Value =
        serde_json::from_str(&stdout).expect("expected valid JSON even for clean file");
    assert_eq!(
        arr.as_array().map(|a| a.len()),
        Some(0),
        "expected empty JSON array for clean file, got: {stdout}"
    );
    let _ = std::fs::remove_file(&src);
}
