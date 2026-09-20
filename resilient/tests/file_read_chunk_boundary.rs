//! RES-4756: file reads must remain bounded before allocation.

fn temp_path(tag: &str) -> String {
    format!("/tmp/resilient-res4756-{tag}-{}", std::process::id())
}

#[test]
fn small_file_reads_preserve_streaming_behavior() {
    let path = temp_path("small");
    std::fs::write(&path, b"abcdef").expect("write fixture");
    let source = format!(
        r#"
fn main() {{
    let file = unwrap(file_open("{path}", "r"));
    let chunk = unwrap(file_read_chunk(file, 3));
    print(bytes_len(chunk));
}}
main();
"#
    );

    let result = resilient::run_program(&source);
    let _ = std::fs::remove_file(&path);
    assert!(result.ok, "small read failed: {:?}", result.errors);
    assert_eq!(result.stdout, "3");
}

#[test]
fn oversized_file_reads_fail_before_backend_access() {
    let path = temp_path("oversized");
    std::fs::write(&path, b"fixture").expect("write fixture");
    let source = format!(
        r#"
fn main() {{
    let file = unwrap(file_open("{path}", "r"));
    let _ = unwrap(file_read_chunk(file, 10485761));
}}
main();
"#
    );

    let result = resilient::run_program(&source);
    let _ = std::fs::remove_file(&path);
    assert!(!result.ok, "oversized read unexpectedly succeeded");
    assert!(
        result.errors.join("\n").contains("too large"),
        "unexpected diagnostics: {:?}",
        result.errors
    );
}
