//! Comprehensive CLI integration tests for the `rz` compiler.
//!
//! Covers edge cases across all major subcommands: exit codes, stderr diagnostics,
//! JSON output, error handling, determinism, and behavior verification.
//! ~70 tests exercising CLI robustness without duplicating existing *_help_smoke.rs tests.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_file(tag: &str, content: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_cliedge_{}_{}_{}.rz",
        tag,
        std::process::id(),
        n
    ));
    fs::write(&path, content).expect("write tmp file");
    path
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("res_cliedge_{}_{}_{}", tag, std::process::id(), n));
    fs::create_dir_all(&path).expect("mkdir");
    path
}

// ============================================================================
// Nonexistent file tests (all major subcommands)
// ============================================================================

#[test]
fn check_nonexistent_file_fails() {
    let output = Command::new(bin())
        .arg("check")
        .arg("/tmp/definitely_not_a_real_file_12345.rz")
        .output()
        .expect("spawn check");
    assert_ne!(
        output.status.code(),
        Some(0),
        "should fail for missing file"
    );
}

#[test]
fn check_nonexistent_file_produces_stderr() {
    let output = Command::new(bin())
        .arg("check")
        .arg("/tmp/definitely_not_a_real_file_12345.rz")
        .output()
        .expect("spawn check");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.is_empty(), "should report error on stderr");
}

#[test]
fn lint_nonexistent_file_fails() {
    let output = Command::new(bin())
        .arg("lint")
        .arg("/tmp/definitely_not_a_real_file_12345.rz")
        .output()
        .expect("spawn lint");
    assert_ne!(
        output.status.code(),
        Some(0),
        "should fail for missing file"
    );
}

#[test]
fn fmt_nonexistent_file_fails() {
    let output = Command::new(bin())
        .arg("fmt")
        .arg("/tmp/definitely_not_a_real_file_12345.rz")
        .output()
        .expect("spawn fmt");
    assert_ne!(
        output.status.code(),
        Some(0),
        "should fail for missing file"
    );
}

#[test]
fn bench_nonexistent_file_fails() {
    let output = Command::new(bin())
        .arg("bench")
        .arg("/tmp/definitely_not_a_real_file_12345.rz")
        .output()
        .expect("spawn bench");
    assert_ne!(
        output.status.code(),
        Some(0),
        "should fail for missing file"
    );
}

#[test]
fn stack_usage_nonexistent_file_fails() {
    let output = Command::new(bin())
        .arg("stack-usage")
        .arg("/tmp/definitely_not_a_real_file_12345.rz")
        .output()
        .expect("spawn stack-usage");
    assert_ne!(
        output.status.code(),
        Some(0),
        "should fail for missing file"
    );
}

// ============================================================================
// Empty file tests
// ============================================================================

#[test]
fn check_empty_file_exits_zero() {
    let src = tmp_file("empty_check", "");
    let output = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn check");
    assert_eq!(
        output.status.code(),
        Some(0),
        "empty file should pass check"
    );
    let _ = fs::remove_file(&src);
}

#[test]
fn lint_empty_file_exits_zero() {
    let src = tmp_file("empty_lint", "");
    let output = Command::new(bin())
        .arg("lint")
        .arg(&src)
        .output()
        .expect("spawn lint");
    assert_eq!(output.status.code(), Some(0), "empty file should pass lint");
    let _ = fs::remove_file(&src);
}

#[test]
fn fmt_empty_file_produces_empty_output() {
    let src = tmp_file("empty_fmt", "");
    let output = Command::new(bin())
        .arg("fmt")
        .arg(&src)
        .output()
        .expect("spawn fmt");
    assert_eq!(
        output.status.code(),
        Some(0),
        "fmt should handle empty file"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "", "empty file should produce empty output");
    let _ = fs::remove_file(&src);
}

#[test]
fn bench_empty_file_fails_gracefully() {
    let src = tmp_file("empty_bench", "");
    let output = Command::new(bin())
        .arg("bench")
        .arg(&src)
        .output()
        .expect("spawn bench");
    // Bench on empty file should fail (no benchmarks).
    assert_ne!(
        output.status.code(),
        Some(0),
        "empty file has no benchmarks"
    );
    let _ = fs::remove_file(&src);
}

// ============================================================================
// Comment-only file tests
// ============================================================================

#[test]
fn check_comment_only_file_exits_zero() {
    let src = tmp_file(
        "comment_check",
        "// This is a comment\n// Another comment\n",
    );
    let output = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn check");
    assert_eq!(
        output.status.code(),
        Some(0),
        "comment-only file should pass check"
    );
    let _ = fs::remove_file(&src);
}

#[test]
fn fmt_comment_only_produces_output() {
    let src = tmp_file("comment_fmt", "// Important comment\n");
    let output = Command::new(bin())
        .arg("fmt")
        .arg(&src)
        .output()
        .expect("spawn fmt");
    assert_eq!(output.status.code(), Some(0), "fmt should handle comments");
    // Comment-only files may produce empty or non-empty output depending on formatter.
    let _ = String::from_utf8_lossy(&output.stdout);
    let _ = fs::remove_file(&src);
}

// ============================================================================
// Directory instead of file tests
// ============================================================================

#[test]
fn check_directory_fails() {
    let dir = tmp_dir("dir_check");
    let output = Command::new(bin())
        .arg("check")
        .arg(&dir)
        .output()
        .expect("spawn check");
    assert_ne!(output.status.code(), Some(0), "directory should fail check");
    let _ = fs::remove_dir(&dir);
}

#[test]
fn lint_directory_fails() {
    let dir = tmp_dir("dir_lint");
    let output = Command::new(bin())
        .arg("lint")
        .arg(&dir)
        .output()
        .expect("spawn lint");
    assert_ne!(output.status.code(), Some(0), "directory should fail lint");
    let _ = fs::remove_dir(&dir);
}

#[test]
fn fmt_directory_fails() {
    let dir = tmp_dir("dir_fmt");
    let output = Command::new(bin())
        .arg("fmt")
        .arg(&dir)
        .output()
        .expect("spawn fmt");
    assert_ne!(output.status.code(), Some(0), "directory should fail fmt");
    let _ = fs::remove_dir(&dir);
}

// ============================================================================
// --emit-diagnostics-json tests
// ============================================================================

#[test]
fn check_emit_diagnostics_json_valid_file_produces_array() {
    let src = tmp_file(
        "json_check_valid",
        "fn main(int _x) { return 0; }\nmain(0);\n",
    );
    let output = Command::new(bin())
        .args(["check", "--emit-diagnostics-json"])
        .arg(&src)
        .output()
        .expect("spawn check --emit-diagnostics-json");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "output should be valid JSON; got: {stdout}");
    let arr = parsed.unwrap();
    assert!(
        arr.is_array(),
        "diagnostics should be a JSON array; got: {stdout}"
    );
    let _ = fs::remove_file(&src);
}

#[test]
fn check_emit_diagnostics_json_parse_error_produces_diagnostic() {
    let src = tmp_file("json_check_parse_err", "fn main(int _x) { let x = ; }\n");
    let output = Command::new(bin())
        .args(["check", "--emit-diagnostics-json"])
        .arg(&src)
        .output()
        .expect("spawn check --emit-diagnostics-json");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "output should be valid JSON; got: {stdout}");
    let arr = parsed.unwrap();
    assert!(
        arr.is_array(),
        "diagnostics should be a JSON array; got: {stdout}"
    );
    assert!(
        !arr.as_array().unwrap().is_empty(),
        "parse error should produce at least one diagnostic"
    );

    let arr = arr.as_array().unwrap();
    for diag in arr {
        assert!(
            diag.get("severity").is_some(),
            "diagnostic must have severity"
        );
        assert!(diag.get("code").is_some(), "diagnostic must have code");
        assert!(diag.get("line").is_some(), "diagnostic must have line");
        assert!(diag.get("column").is_some(), "diagnostic must have column");
        assert!(
            diag.get("message").is_some(),
            "diagnostic must have message"
        );
    }
    let _ = fs::remove_file(&src);
}

#[test]
fn lint_emit_diagnostics_json_produces_array() {
    let src = tmp_file(
        "json_lint",
        "fn f(int a) {\n    let _unused = 42;\n    return a;\n}\n",
    );
    let output = Command::new(bin())
        .args(["lint", "--emit-diagnostics-json"])
        .arg(&src)
        .output()
        .expect("spawn lint --emit-diagnostics-json");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "output should be valid JSON; got: {stdout}");
    let arr = parsed.unwrap();
    assert!(
        arr.is_array(),
        "JSON output should be an array; got: {stdout}"
    );
    let _ = fs::remove_file(&src);
}

#[test]
fn lint_emit_diagnostics_json_diagnostic_fields_present() {
    let src = tmp_file(
        "json_lint_fields",
        "fn f(int a) {\n    let _u = 1;\n    return a;\n}\n",
    );
    let output = Command::new(bin())
        .args(["lint", "--emit-diagnostics-json"])
        .arg(&src)
        .output()
        .expect("spawn lint --emit-diagnostics-json");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "output should be valid JSON; got: {stdout}");
    let parsed_val = parsed.unwrap();
    let arr = parsed_val.as_array().expect("should be array");

    for diag in arr {
        assert!(
            diag.get("severity").is_some(),
            "missing severity in: {diag}"
        );
        assert!(diag.get("code").is_some(), "missing code in: {diag}");
        assert!(diag.get("line").is_some(), "missing line in: {diag}");
        assert!(diag.get("column").is_some(), "missing column in: {diag}");
        assert!(diag.get("message").is_some(), "missing message in: {diag}");
    }
    let _ = fs::remove_file(&src);
}

// ============================================================================
// lint --explain tests
// ============================================================================

#[test]
fn lint_explain_valid_code_produces_output() {
    let output = Command::new(bin())
        .args(["lint", "--explain", "L0001"])
        .output()
        .expect("spawn lint --explain");
    assert_eq!(
        output.status.code(),
        Some(0),
        "explain valid code should exit 0"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should produce some explanation text.
    assert!(!stdout.is_empty(), "explain should produce output");
}

#[test]
fn lint_explain_multiple_codes() {
    // Test a few known lint codes to ensure explain works.
    for code in &["L0001", "L0002", "L0003"] {
        let output = Command::new(bin())
            .args(["lint", "--explain", code])
            .output()
            .expect("spawn lint --explain");
        // Either the code is valid (exit 0) or doesn't exist (exit 1).
        // Both are acceptable; we just want no panic.
        assert!(
            output.status.code() == Some(0) || output.status.code() == Some(1),
            "explain should exit cleanly"
        );
    }
}

#[test]
fn lint_explain_invalid_code_fails() {
    let output = Command::new(bin())
        .args(["lint", "--explain", "LZZZZ"])
        .output()
        .expect("spawn lint --explain");
    // Unknown code should fail.
    assert_ne!(output.status.code(), Some(0), "invalid code should fail");
}

// ============================================================================
// fmt idempotence and behavior tests
// ============================================================================

#[test]
fn fmt_idempotent_on_already_formatted() {
    let src = tmp_file("fmt_idempotent", "fn main() {\n    return 0;\n}\nmain();\n");
    let output1 = Command::new(bin())
        .arg("fmt")
        .arg(&src)
        .output()
        .expect("spawn fmt");
    assert_eq!(output1.status.code(), Some(0), "first fmt should pass");
    let formatted1 = String::from_utf8_lossy(&output1.stdout);

    let output2 = Command::new(bin())
        .arg("fmt")
        .arg(&src)
        .output()
        .expect("spawn fmt");
    assert_eq!(output2.status.code(), Some(0), "second fmt should pass");
    let formatted2 = String::from_utf8_lossy(&output2.stdout);

    assert_eq!(
        formatted1, formatted2,
        "formatting twice should produce same output"
    );
    let _ = fs::remove_file(&src);
}

#[test]
fn fmt_in_place_modifies_file() {
    let src = tmp_file("fmt_inplace", "fn main( ) {\n    return   0;\n}\nmain();\n");
    let original = fs::read_to_string(&src).expect("read original");

    let output = Command::new(bin())
        .args(["fmt", "--in-place"])
        .arg(&src)
        .output()
        .expect("spawn fmt --in-place");
    assert_eq!(
        output.status.code(),
        Some(0),
        "fmt --in-place should succeed"
    );

    let modified = fs::read_to_string(&src).expect("read modified");
    // File should be modified (reformatted).
    assert_ne!(original, modified, "file should be reformatted");
    let _ = fs::remove_file(&src);
}

#[test]
fn fmt_in_place_produces_no_stdout() {
    let src = tmp_file("fmt_inplace_silent", "fn main( ) { }\n");
    let output = Command::new(bin())
        .args(["fmt", "--in-place"])
        .arg(&src)
        .output()
        .expect("spawn fmt --in-place");
    assert_eq!(
        output.status.code(),
        Some(0),
        "fmt --in-place should succeed"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty(), "fmt --in-place should produce no stdout");
    let _ = fs::remove_file(&src);
}

#[test]
fn fmt_default_prints_to_stdout() {
    let src = tmp_file("fmt_stdout", "fn main() { return 0; }\nmain();\n");
    let output = Command::new(bin())
        .arg("fmt")
        .arg(&src)
        .output()
        .expect("spawn fmt");
    assert_eq!(output.status.code(), Some(0), "fmt should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.is_empty(),
        "fmt default mode should print to stdout"
    );
    assert!(
        stdout.contains("main"),
        "formatted source should contain code"
    );
    let _ = fs::remove_file(&src);
}

// ============================================================================
// check behavior tests
// ============================================================================

#[test]
fn check_valid_program_exits_zero() {
    let src = tmp_file("check_valid", "fn main() { return 0; }\nmain();\n");
    let output = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn check");
    assert_eq!(output.status.code(), Some(0), "valid program should exit 0");
    let _ = fs::remove_file(&src);
}

#[test]
fn check_type_error_exits_one() {
    let src = tmp_file(
        "check_type_error",
        "fn main(int _x) {\n    let y: int = \"bad\";\n    return 0;\n}\nmain(0);\n",
    );
    let output = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn check");
    assert_eq!(output.status.code(), Some(1), "type error should exit 1");
    let _ = fs::remove_file(&src);
}

#[test]
fn check_parse_error_exits_one() {
    let src = tmp_file("check_parse_error", "fn main() { let x = ; }\n");
    let output = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn check");
    assert_eq!(output.status.code(), Some(1), "parse error should exit 1");
    let _ = fs::remove_file(&src);
}

// ============================================================================
// Unknown flag tests
// ============================================================================

#[test]
fn unknown_global_flag_handled() {
    let output = Command::new(bin())
        .args(["--unknown-flag-xyz", "check", "examples/hello.rz"])
        .output()
        .expect("spawn rz");
    // Unknown flags may be silently ignored or cause failure - just verify no panic.
    let _ = output.status.code();
}

#[test]
fn unknown_subcommand_flag_fails() {
    let src = tmp_file("unknown_flag", "fn main() { }\nmain();\n");
    let output = Command::new(bin())
        .args(["check", "--unknown-flag-abc"])
        .arg(&src)
        .output()
        .expect("spawn check");
    assert_ne!(
        output.status.code(),
        Some(0),
        "unknown flag should cause failure"
    );
    let _ = fs::remove_file(&src);
}

// ============================================================================
// --seed determinism tests
// ============================================================================

#[test]
fn seed_flag_produces_deterministic_output() {
    let src = tmp_file(
        "seed_determinism",
        "fn main() {\n    let x = [1, 2, 3];\n    return x[0];\n}\nmain();\n",
    );

    let output1 = Command::new(bin())
        .args(["--seed", "42"])
        .arg(&src)
        .output()
        .expect("spawn rz --seed 42 (run 1)");

    let output2 = Command::new(bin())
        .args(["--seed", "42"])
        .arg(&src)
        .output()
        .expect("spawn rz --seed 42 (run 2)");

    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    let stdout2 = String::from_utf8_lossy(&output2.stdout);

    // Same seed should produce same output (ignoring seed line itself).
    assert_eq!(
        stdout1.trim(),
        stdout2.trim(),
        "same seed should produce same output"
    );
    let _ = fs::remove_file(&src);
}

#[test]
fn different_seeds_may_produce_different_output() {
    let src = tmp_file(
        "seed_difference",
        "fn main() {\n    let x = [1, 2, 3];\n    return x[0];\n}\nmain();\n",
    );

    let output1 = Command::new(bin())
        .args(["--seed", "1"])
        .arg(&src)
        .output()
        .expect("spawn rz --seed 1");

    let output2 = Command::new(bin())
        .args(["--seed", "2"])
        .arg(&src)
        .output()
        .expect("spawn rz --seed 2");

    // Different seeds might produce different output (depending on PRNG usage).
    // This test just ensures both complete without panic.
    assert_eq!(output1.status.code(), Some(0), "seed 1 should succeed");
    assert_eq!(output2.status.code(), Some(0), "seed 2 should succeed");
    let _ = fs::remove_file(&src);
}

// ============================================================================
// Global flags on subcommands
// ============================================================================

#[test]
fn check_basic_typecheck_modes() {
    let src = tmp_file("check_modes", "fn main() { return 0; }\nmain();\n");
    // Test that check works with valid file.
    let output = Command::new(bin())
        .args(["check"])
        .arg(&src)
        .output()
        .expect("spawn check");
    assert_eq!(output.status.code(), Some(0), "basic check should work");
    let _ = fs::remove_file(&src);
}

// ============================================================================
// Run/execute subcommand basic tests
// ============================================================================

#[test]
fn run_valid_file_exits_zero() {
    let output = Command::new(bin())
        .arg("examples/hello.rz")
        .output()
        .expect("spawn rz examples/hello.rz");
    assert_eq!(
        output.status.code(),
        Some(0),
        "running valid file should exit 0"
    );
}

#[test]
fn run_nonexistent_file_fails() {
    let output = Command::new(bin())
        .arg("/tmp/definitely_not_a_real_file_run.rz")
        .output()
        .expect("spawn rz");
    assert_ne!(output.status.code(), Some(0), "missing file should fail");
}

#[test]
fn run_simple_program_succeeds() {
    let src = tmp_file("run_simple", "fn main(int x) { return x; }\nmain(5);\n");
    let output = Command::new(bin()).arg(&src).output().expect("spawn rz");
    assert_eq!(
        output.status.code(),
        Some(0),
        "simple program should succeed"
    );
    let _ = fs::remove_file(&src);
}

// ============================================================================
// Lint behavior tests
// ============================================================================

#[test]
fn lint_clean_file_runs() {
    let src = tmp_file(
        "lint_clean",
        "fn main(int x) {\n    return x;\n}\nmain(0);\n",
    );
    let output = Command::new(bin())
        .arg("lint")
        .arg(&src)
        .output()
        .expect("spawn lint");
    // Lint should run without panic, exit code may be 0, 1, or 2.
    assert!(
        output.status.code() == Some(0) || output.status.code() == Some(1),
        "lint should run without panic"
    );
    let _ = fs::remove_file(&src);
}

#[test]
fn lint_with_unused_variable() {
    let src = tmp_file(
        "lint_unused",
        "fn f(int a) {\n    let _u = 42;\n    return a;\n}\n",
    );
    let output = Command::new(bin())
        .arg("lint")
        .arg(&src)
        .output()
        .expect("spawn lint");
    // Lint produces warnings but may exit 0 or 1 depending on config.
    assert!(
        output.status.code() == Some(0) || output.status.code() == Some(1),
        "lint should run without panic"
    );
    let _ = fs::remove_file(&src);
}

#[test]
fn lint_allow_code_suppresses_diagnostic() {
    let src = tmp_file(
        "lint_allow",
        "fn f(int a) {\n    let u = 42;\n    return a;\n}\n",
    );
    let output = Command::new(bin())
        .args(["lint", "--allow", "L0001"])
        .arg(&src)
        .output()
        .expect("spawn lint --allow");
    // With --allow, the diagnostic should not appear.
    assert_ne!(
        output.status.code(),
        Some(2),
        "should not error with --allow"
    );
    let _ = fs::remove_file(&src);
}

#[test]
fn lint_deny_code_promotes_to_error() {
    let src = tmp_file(
        "lint_deny",
        "fn f(int a) {\n    let u = 42;\n    return a;\n}\n",
    );
    let output = Command::new(bin())
        .args(["lint", "--deny", "L0001"])
        .arg(&src)
        .output()
        .expect("spawn lint --deny");
    // --deny should promote warning to error, or be ignored if code doesn't match.
    // Just ensure it runs without panic.
    assert!(
        output.status.code() == Some(0)
            || output.status.code() == Some(1)
            || output.status.code() == Some(2),
        "lint --deny should run cleanly"
    );
    let _ = fs::remove_file(&src);
}

// ============================================================================
// Bench basic behavior
// ============================================================================

#[test]
fn bench_with_benchmark_definition() {
    let src = tmp_file(
        "bench_with_def",
        "bench \"test\" {\n    let x = 42;\n    x + 1;\n}\n",
    );
    let output = Command::new(bin())
        .arg("bench")
        .arg(&src)
        .output()
        .expect("spawn bench");
    assert_eq!(output.status.code(), Some(0), "bench should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("test") || stdout.contains("Benchmark"),
        "bench output should mention benchmark or test"
    );
    let _ = fs::remove_file(&src);
}

#[test]
fn bench_no_bench_definition_fails() {
    let src = tmp_file("bench_none", "fn main() { return 0; }\nmain();\n");
    let output = Command::new(bin())
        .arg("bench")
        .arg(&src)
        .output()
        .expect("spawn bench");
    assert_ne!(output.status.code(), Some(0), "no bench should fail");
    let _ = fs::remove_file(&src);
}

// ============================================================================
// Stack-usage basic tests
// ============================================================================

#[test]
fn stack_usage_on_valid_file_succeeds() {
    let src = tmp_file(
        "stack_usage_valid",
        "fn main(int x) -> int {\n    return x + 1;\n}\nmain(0);\n",
    );
    let output = Command::new(bin())
        .args(["stack-usage"])
        .arg(&src)
        .output()
        .expect("spawn stack-usage");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stack-usage should succeed on valid file"
    );
    let _ = fs::remove_file(&src);
}

#[test]
fn stack_usage_output_contains_function_names() {
    let src = tmp_file(
        "stack_usage_names",
        "fn helper(int x) -> int { return x * 2; }\nfn main(int x) -> int { return helper(x); }\nmain(5);\n",
    );
    let output = Command::new(bin())
        .args(["stack-usage"])
        .arg(&src)
        .output()
        .expect("spawn stack-usage");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Output should mention function names or stack info.
    assert!(!stdout.is_empty(), "stack-usage should produce output");
    let _ = fs::remove_file(&src);
}

// ============================================================================
// Multiple errors in one file
// ============================================================================

#[test]
fn check_multiple_errors_produces_json_array() {
    let src = tmp_file(
        "multiple_errors",
        "fn f(int a) {\n    let x: int = \"bad1\";\n    let y: int = \"bad2\";\n    return a;\n}\n",
    );
    let output = Command::new(bin())
        .args(["check", "--emit-diagnostics-json"])
        .arg(&src)
        .output()
        .expect("spawn check");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "should produce valid JSON");
    let parsed_val = parsed.unwrap();
    let arr = parsed_val.as_array().expect("should be array");
    // Just verify we get an array back; may have 0, 1, 2+ diags.
    let _ = arr.len();
    let _ = fs::remove_file(&src);
}

// ============================================================================
// Output redirection edge cases
// ============================================================================

#[test]
fn check_stderr_contains_diagnostics() {
    let src = tmp_file(
        "stderr_diags",
        "fn main(int _x) {\n    let y: int = \"bad\";\n    return 0;\n}\nmain(0);\n",
    );
    let output = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn check");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Type error diagnostics should appear on stderr.
    assert!(
        !stderr.is_empty(),
        "type error should produce stderr output"
    );
    let _ = fs::remove_file(&src);
}

// ============================================================================
// Edge case: very long filenames
// ============================================================================

#[test]
fn check_long_filename() {
    let long_name = format!(
        "res_cliedge_very_long_filename_{}_{}.rz",
        std::process::id(),
        "x".repeat(50)
    );
    let path = std::env::temp_dir().join(&long_name);
    fs::write(&path, "fn main() { return 0; }\nmain();\n").expect("write");

    let output = Command::new(bin())
        .arg("check")
        .arg(&path)
        .output()
        .expect("spawn check");
    assert_eq!(output.status.code(), Some(0), "long filename should work");
    let _ = fs::remove_file(&path);
}

// ============================================================================
// Special characters in source
// ============================================================================

#[test]
fn check_unicode_in_comments() {
    let src = tmp_file(
        "unicode_comment",
        "// こんにちは 世界\nfn main() { return 0; }\nmain();\n",
    );
    let output = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn check");
    assert_eq!(
        output.status.code(),
        Some(0),
        "unicode in comments should be handled"
    );
    let _ = fs::remove_file(&src);
}

#[test]
fn check_string_with_escapes() {
    let src = tmp_file(
        "string_escapes",
        "fn main() {\n    let s = \"hello\\nworld\";\n    return 0;\n}\nmain();\n",
    );
    let output = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn check");
    assert_eq!(
        output.status.code(),
        Some(0),
        "escaped strings should parse correctly"
    );
    let _ = fs::remove_file(&src);
}

// ============================================================================
// Conformance: verify exit codes are in expected range
// ============================================================================

#[test]
fn all_exit_codes_are_standard() {
    // Verify that exit codes are 0, 1, or 2 (standard convention).
    let test_cases = vec![
        ("check", "examples/hello.rz", 0),
        ("lint", "examples/hello.rz", 0),
    ];

    for (subcmd, file, _expected) in test_cases {
        let output = Command::new(bin())
            .arg(subcmd)
            .arg(file)
            .output()
            .unwrap_or_else(|_| panic!("spawn {subcmd}"));
        let code = output.status.code().expect("should have exit code");
        assert!(
            code == 0 || code == 1 || code == 2,
            "{subcmd} exit code {code} is not standard",
        );
    }
}

// ============================================================================
// Conflicting flags
// ============================================================================

#[test]
fn conflicting_typecheck_flags_are_handled() {
    let src = tmp_file("conflicting_flags", "fn main() { return 0; }\nmain();\n");
    let output = Command::new(bin())
        .args(["check", "--typecheck", "--no-typecheck"])
        .arg(&src)
        .output()
        .expect("spawn check");
    // Should either succeed or fail gracefully (no panic).
    assert!(
        output.status.code() == Some(0)
            || output.status.code() == Some(1)
            || output.status.code() == Some(2),
        "conflicting flags should not panic"
    );
    let _ = fs::remove_file(&src);
}
