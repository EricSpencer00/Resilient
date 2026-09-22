//! Regression coverage for the empty-input bytes_repeat resource boundary.

#[test]
fn bytes_repeat_empty_input_returns_for_maximum_count() {
    let result = resilient::run_program(
        r#"
fn main() {
    let repeated = bytes_repeat(b"", 9223372036854775807);
    print(bytes_len(repeated));
}
main();
"#,
    );

    assert!(result.ok, "errors: {:?}", result.errors);
    assert_eq!(result.stdout.trim(), "0");
}

#[test]
fn bytes_repeat_nonempty_input_still_enforces_length_cap() {
    let result = resilient::run_program(
        r#"
fn main() {
    bytes_repeat(b"x", 2000000000);
}
main();
"#,
    );

    assert!(!result.ok, "oversized repetition unexpectedly succeeded");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("exceed cap"))
    );
}
