//! RES-4756: file reads must remain bounded before allocation.

fn temp_path(tag: &str) -> String {
    format!("/tmp/resilient-res4756-{tag}-{}", std::process::id())
}

fn run_read(path: &str, body: &str) -> resilient::RunResult {
    let source = format!(
        r#"
fn main() {{
    let file = unwrap(file_open("{path}", "r"));
    {body}
}}
main();
"#
    );
    resilient::run_program(&source)
}

#[test]
fn small_file_reads_preserve_streaming_behavior() {
    let path = temp_path("small");
    std::fs::write(&path, b"abcdef").expect("write fixture");
    let result = run_read(
        &path,
        "let chunk = unwrap(file_read_chunk(file, 3));\nprint(bytes_len(chunk));",
    );
    let _ = std::fs::remove_file(&path);
    assert!(result.ok, "small read failed: {:?}", result.errors);
    assert_eq!(result.stdout, "3");
}

#[test]
fn oversized_file_reads_fail_before_backend_access() {
    let path = temp_path("oversized");
    std::fs::write(&path, b"fixture").expect("write fixture");
    let result = run_read(
        &path,
        "file_close(file);\nlet _ = unwrap(file_read_chunk(file, 10485761));",
    );
    let _ = std::fs::remove_file(&path);
    assert!(!result.ok, "oversized read unexpectedly succeeded");
    assert!(
        result.errors.join("\n").contains("too large"),
        "unexpected diagnostics: {:?}",
        result.errors
    );
}

#[test]
fn i64_max_read_is_rejected_before_narrowing() {
    let path = temp_path("width");
    std::fs::write(&path, b"fixture").expect("write fixture");
    let result = run_read(
        &path,
        "let _ = unwrap(file_read_chunk(file, 9223372036854775807));",
    );
    let _ = std::fs::remove_file(&path);

    assert!(!result.ok, "i64::MAX read unexpectedly succeeded");
    assert!(
        result.errors.join("\n").contains("too large"),
        "unexpected diagnostics: {:?}",
        result.errors
    );
}

#[test]
fn negative_read_keeps_existing_diagnostic() {
    let path = temp_path("negative");
    std::fs::write(&path, b"fixture").expect("write fixture");
    let result = run_read(&path, "let _ = unwrap(file_read_chunk(file, -1));");
    let _ = std::fs::remove_file(&path);

    assert!(!result.ok, "negative read unexpectedly succeeded");
    assert!(
        result
            .errors
            .join("\n")
            .contains("max_bytes must be non-negative"),
        "unexpected diagnostics: {:?}",
        result.errors
    );
}
