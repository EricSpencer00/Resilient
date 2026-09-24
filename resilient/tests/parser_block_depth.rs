//! End-to-end coverage for structurally nested parser blocks.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const MAX_BLOCK_DEPTH: usize = 256;

struct TempSource(PathBuf);

impl Drop for TempSource {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn nested_if_source(depth: usize) -> String {
    let mut source = String::from("fn main() {\n");
    for _ in 0..depth {
        source.push_str("if true {\n");
    }
    for _ in 0..depth {
        source.push_str("}\n");
    }
    source.push_str("}\n");
    source
}

fn run_source(source: &str) -> Output {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "res_4871_parser_blocks_{}_{}",
        std::process::id(),
        id
    ));
    std::fs::create_dir(&dir).expect("create temporary source directory");
    let _cleanup = TempSource(dir.clone());
    let source_path = dir.join("nested_blocks.rz");
    std::fs::write(&source_path, source).expect("write nested-block source");
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

#[test]
fn source_at_the_supported_block_depth_still_typechecks() {
    let output = run_source(&nested_if_source(MAX_BLOCK_DEPTH - 1));
    assert!(
        output.status.success(),
        "supported nesting should typecheck: {}",
        diagnostics(&output)
    );
}

#[test]
fn first_block_beyond_limit_reports_a_positioned_parser_error() {
    let output = run_source(&nested_if_source(MAX_BLOCK_DEPTH));
    let text = diagnostics(&output);
    assert!(
        !output.status.success() && output.status.code().is_some(),
        "over-limit input should exit normally with an error, not a signal: {text}"
    );
    assert!(
        text.contains("block nesting too deep"),
        "expected the structural depth diagnostic: {text}"
    );
    assert!(
        text.contains(&format!("{}:", MAX_BLOCK_DEPTH + 1)),
        "expected a line:column source position for the rejected block: {text}"
    );
}

#[test]
fn hostile_block_nesting_does_not_abort_the_compiler_process() {
    let output = run_source(&nested_if_source(20_000));
    let text = diagnostics(&output);
    assert!(
        !output.status.success() && output.status.code().is_some(),
        "hostile nesting should exit normally with an error, not a signal: {text}"
    );
    assert!(
        text.contains("block nesting too deep"),
        "expected bounded parser rejection: {text}"
    );
}

const MAX_IF_CHAIN_DEPTH: usize = 256;

fn else_if_chain_source(depth: usize) -> String {
    let mut source = String::from("fn main() {\n");
    for index in 0..depth {
        if index == 0 {
            source.push_str("if true {}\n");
        } else {
            source.push_str("else if true {}\n");
        }
    }
    source.push_str("else {}\n}\n");
    source
}

#[test]
fn source_at_the_supported_else_if_depth_still_typechecks() {
    let output = run_source(&else_if_chain_source(MAX_IF_CHAIN_DEPTH));
    assert!(
        output.status.success(),
        "supported else-if nesting should typecheck: {}",
        diagnostics(&output)
    );
}

#[test]
fn first_else_if_beyond_limit_reports_a_positioned_parser_error() {
    let output = run_source(&else_if_chain_source(MAX_IF_CHAIN_DEPTH + 1));
    let text = diagnostics(&output);
    assert!(
        !output.status.success() && output.status.code().is_some(),
        "over-limit else-if input should exit normally with an error, not a signal: {text}"
    );
    assert!(
        text.contains("conditional nesting too deep"),
        "expected the conditional depth diagnostic: {text}"
    );
    assert!(
        text.contains("1 parser error(s)"),
        "over-limit else-if recovery should avoid diagnostic cascades: {text}"
    );
    assert!(
        text.contains(&format!("{}:", MAX_IF_CHAIN_DEPTH + 2)),
        "expected a line:column source position for the rejected conditional: {text}"
    );
}

#[test]
fn hostile_else_if_nesting_does_not_abort_the_compiler_process() {
    let output = run_source(&else_if_chain_source(20_000));
    let text = diagnostics(&output);
    assert!(
        !output.status.success() && output.status.code().is_some(),
        "hostile else-if input should exit normally with an error, not a signal: {text}"
    );
    assert!(
        text.contains("conditional nesting too deep"),
        "expected bounded parser rejection: {text}"
    );
}

fn over_limit_block_with_brace_literals() -> String {
    let mut source = String::from("fn main() {\n");
    for _ in 0..MAX_BLOCK_DEPTH {
        source.push_str("if true {\n");
    }
    source.push_str("let set_value = #{1};\n");
    source.push_str("let text_value = \"{ not a block }\";\n");
    source.push_str("let map_value = {\"key\" -> 1};\n");
    for _ in 0..MAX_BLOCK_DEPTH {
        source.push_str("}\n");
    }
    source.push_str("}\nfn after_depth_error() {}\n");
    source
}

fn unterminated_over_limit_block() -> String {
    let mut source = String::from("fn main() {\n");
    for _ in 0..MAX_BLOCK_DEPTH {
        source.push_str("if true {\n");
    }
    source.push_str("let unfinished = #{1};\n");
    source
}

#[test]
fn iterative_recovery_counts_map_and_set_brace_tokens() {
    let output = run_source(&over_limit_block_with_brace_literals());
    let text = diagnostics(&output);
    assert!(
        !output.status.success() && output.status.code().is_some(),
        "over-limit literal input should exit normally with a parser error: {text}"
    );
    assert!(
        text.contains("block nesting too deep"),
        "expected the block depth diagnostic: {text}"
    );
    assert!(
        text.contains("Failed to parse program: 1 parser error(s)"),
        "recovery should produce one diagnostic and preserve the following function: {text}"
    );
}

#[test]
fn iterative_recovery_terminates_when_the_rejected_block_reaches_eof() {
    let output = run_source(&unterminated_over_limit_block());
    let text = diagnostics(&output);
    assert!(
        !output.status.success() && output.status.code().is_some(),
        "unterminated over-limit input should exit normally with a parser error: {text}"
    );
    assert!(
        text.contains("block nesting too deep"),
        "expected the block depth diagnostic before EOF recovery: {text}"
    );
}
