//! Public-path coverage for recursive type-annotation parsing limits.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const MAX_TYPE_DEPTH: usize = 256;

struct TempSource(PathBuf);

impl Drop for TempSource {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn nested_option_source(depth: usize) -> String {
    let ty = format!("{}Int{}", "Option<".repeat(depth), ">".repeat(depth));
    format!("fn accepts({ty} value) -> Int {{ return 0; }}\n")
}

fn nested_reference_source(depth: usize) -> String {
    let ty = format!("{}Int", "& ".repeat(depth));
    format!("fn accepts({ty} value) -> Int {{ return 0; }}\n")
}

fn nested_option_alias_source(depth: usize) -> String {
    let ty = format!("{}Int{}", "Option<".repeat(depth), ">".repeat(depth));
    format!("type Deep = {ty};\n")
}

fn nested_reference_alias_source(depth: usize) -> String {
    let ty = format!("{}Int", "& ".repeat(depth));
    format!("type Deep = {ty};\n")
}

fn with_following_function(mut source: String) -> String {
    source.push_str("fn after_depth_error() {}\n");
    source
}

fn run_source(source: &str) -> Output {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("res_4875_type_depth_{}_{}", std::process::id(), id));
    std::fs::create_dir(&dir).expect("create temporary source directory");
    let _cleanup = TempSource(dir.clone());
    let source_path = dir.join("nested_types.rz");
    std::fs::write(&source_path, source).expect("write nested-type source");
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
        text.contains("type annotation nesting too deep"),
        "expected the parser depth diagnostic: {text}"
    );
    assert!(
        text.contains("1:"),
        "expected a line:column source position for the rejected type: {text}"
    );
    assert!(
        text.contains("1 parser error(s)"),
        "recovery should report one parser error without a cascade: {text}"
    );
}

#[test]
fn nested_generic_at_supported_boundary_still_typechecks() {
    let output = run_source(&nested_option_source(MAX_TYPE_DEPTH - 1));
    assert!(
        output.status.success(),
        "supported generic nesting should typecheck: {}",
        diagnostics(&output)
    );
}

#[test]
fn first_nested_generic_beyond_limit_reports_a_positioned_error() {
    let source = with_following_function(nested_option_alias_source(MAX_TYPE_DEPTH));
    let output = run_source(&source);
    let text = diagnostics(&output);
    assert_positioned_depth_error(&output, &text, "generic nesting");
}

#[test]
fn nested_references_at_supported_boundary_still_typecheck() {
    let output = run_source(&nested_reference_source(MAX_TYPE_DEPTH - 1));
    assert!(
        output.status.success(),
        "supported reference nesting should typecheck: {}",
        diagnostics(&output)
    );
}

#[test]
fn first_nested_reference_beyond_limit_reports_a_positioned_error() {
    let source = with_following_function(nested_reference_alias_source(MAX_TYPE_DEPTH));
    let output = run_source(&source);
    let text = diagnostics(&output);
    assert_positioned_depth_error(&output, &text, "reference nesting");
}

#[test]
fn hostile_generic_nesting_does_not_abort_the_compiler_process() {
    let source = with_following_function(nested_option_alias_source(20_000));
    let output = run_source(&source);
    let text = diagnostics(&output);
    assert_positioned_depth_error(&output, &text, "hostile generic nesting");
}

#[test]
fn hostile_reference_nesting_does_not_abort_the_compiler_process() {
    let source = with_following_function(nested_reference_alias_source(20_000));
    let output = run_source(&source);
    let text = diagnostics(&output);
    assert_positioned_depth_error(&output, &text, "hostile reference nesting");
}
