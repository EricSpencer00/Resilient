//! Resource-boundary regressions for the user-controlled matrix constructor.

#[test]
fn mat_identity_rejects_dimensions_above_the_cell_budget() {
    let result = resilient::run_program("let matrix = mat_identity(1001);");

    assert!(
        !result.ok,
        "oversized identity matrix unexpectedly succeeded"
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("mat_identity") && error.contains("maximum")),
        "missing matrix budget diagnostic: {:?}",
        result.errors
    );
}

#[test]
fn mat_identity_rejects_dimensions_that_overflow_cell_count() {
    let result = resilient::run_program("let matrix = mat_identity(9223372036854775807);");

    assert!(
        !result.ok,
        "overflowing identity matrix unexpectedly succeeded"
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("mat_identity")
                && (error.contains("overflows") || error.contains("target platform"))),
        "missing matrix overflow diagnostic: {:?}",
        result.errors
    );
}
