//! Public CLI coverage for recursive match-pattern parsing limits.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const MAX_PATTERN_DEPTH: usize = 256;

struct TempSource(PathBuf);

impl Drop for TempSource {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn nested_parenthesized_pattern(depth: usize) -> String {
    let mut pattern = "(".repeat(depth);
    pattern.push('_');
    pattern.push_str(&")".repeat(depth));
    pattern
}

fn nested_some_pattern(depth: usize) -> String {
    let mut pattern = "Some(".repeat(depth);
    pattern.push('_');
    pattern.push_str(&")".repeat(depth));
    pattern
}

fn nested_enum_payload_pattern(depth: usize) -> String {
    let mut pattern = "E::V(".repeat(depth);
    pattern.push('_');
    pattern.push_str(&")".repeat(depth));
    pattern
}

fn nested_binding_pattern(depth: usize) -> String {
    format!("{}_{}", "item @ ".repeat(depth), "_")
}

fn source_with_pattern(pattern: &str) -> String {
    format!(
        "fn accepts(Int value) -> Int {{\n  return match value {{\n    {pattern} => 1,\n    _ => 0\n  }};\n}}\nfn after_depth_error() -> Int {{ return 0; }}\n"
    )
}

fn run_source(source: &str) -> Output {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "res_4877_pattern_depth_{}_{}",
        std::process::id(),
        id
    ));
    std::fs::create_dir(&dir).expect("create temporary source directory");
    let _cleanup = TempSource(dir.clone());
    let source_path = dir.join("nested_patterns.rz");
    std::fs::write(&source_path, source).expect("write nested-pattern source");
    cli_command(&source_path)
        .current_dir(&dir)
        .output()
        .expect("run rz --typecheck-strict")
}

#[cfg(unix)]
fn cli_command(source_path: &Path) -> Command {
    let mut command = Command::new("sh");
    command
        .args(["-c", "ulimit -c 0; exec \"$@\"", "rz"])
        .arg(env!("CARGO_BIN_EXE_rz"))
        .arg("--typecheck-strict")
        .arg(source_path);
    command
}

#[cfg(not(unix))]
fn cli_command(source_path: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
    command.arg("--typecheck-strict").arg(source_path);
    command
}

fn diagnostics(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_positioned_depth_error(output: &Output, text: &str, kind: &str) {
    assert!(
        !output.status.success() && output.status.code().is_some(),
        "over-limit {kind} should exit normally with an error, not a signal: {text}"
    );
    assert!(
        text.contains("pattern nesting too deep (limit 256)"),
        "expected the parser depth diagnostic: {text}"
    );
    assert!(
        text.contains("3:"),
        "expected a line:column source position for the rejected pattern: {text}"
    );
    assert!(
        text.contains("1 parser error(s)"),
        "recovery should report one parser error without a cascade: {text}"
    );
}

#[test]
fn nested_parenthesized_pattern_at_supported_boundary_still_typechecks() {
    let source = source_with_pattern(&nested_parenthesized_pattern(MAX_PATTERN_DEPTH - 1));
    let output = run_source(&source);
    assert!(
        output.status.success(),
        "supported pattern nesting should typecheck: {}",
        diagnostics(&output)
    );
}

#[test]
fn first_parenthesized_pattern_beyond_limit_reports_positioned_error() {
    let source = source_with_pattern(&nested_parenthesized_pattern(MAX_PATTERN_DEPTH));
    let output = run_source(&source);
    let text = diagnostics(&output);
    assert_positioned_depth_error(&output, &text, "parenthesized pattern");
}

#[test]
fn hostile_parenthesized_pattern_does_not_abort_the_compiler_process() {
    let source = source_with_pattern(&nested_parenthesized_pattern(20_000));
    let output = run_source(&source);
    let text = diagnostics(&output);
    assert_positioned_depth_error(&output, &text, "hostile parenthesized pattern");
}

#[test]
fn hostile_variant_payload_pattern_does_not_abort_the_compiler_process() {
    let source = source_with_pattern(&nested_some_pattern(20_000));
    let output = run_source(&source);
    let text = diagnostics(&output);
    assert_positioned_depth_error(&output, &text, "hostile Some pattern");
}

#[test]
fn hostile_enum_payload_pattern_does_not_abort_the_compiler_process() {
    let source = source_with_pattern(&nested_enum_payload_pattern(20_000));
    let output = run_source(&source);
    let text = diagnostics(&output);
    assert_positioned_depth_error(&output, &text, "hostile enum payload pattern");
}

#[test]
fn hostile_binding_pattern_does_not_abort_the_compiler_process() {
    let source = source_with_pattern(&nested_binding_pattern(20_000));
    let output = run_source(&source);
    let text = diagnostics(&output);
    assert_positioned_depth_error(&output, &text, "hostile binding pattern");
}
