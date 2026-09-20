//! Regression coverage for the typed-buffer allocation budget.

#[test]
fn oversized_integer_buffer_is_rejected_before_allocation() {
    let result = resilient::run_program(
        r#"
let buffer = buffer_int(10_000_001)
"#,
    );

    assert!(!result.ok, "oversized buffer_int should fail");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("buffer_int: length 10000001 too large")),
        "unexpected diagnostics: {:?}",
        result.errors
    );
}

#[test]
fn oversized_float_buffer_is_rejected_before_allocation() {
    let result = resilient::run_program(
        r#"
let buffer = buffer_float(10_000_001)
"#,
    );

    assert!(!result.ok, "oversized buffer_float should fail");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("buffer_float: length 10000001 too large")),
        "unexpected diagnostics: {:?}",
        result.errors
    );
}
