//! Regression coverage for the `round_to` precision boundary.

#[test]
fn round_to_rejects_precision_that_pow_i_cannot_represent() {
    let result = resilient::run_program(
        "fn main() { println(round_to(1.25, 9223372036854775807)); } main();",
    );
    assert!(!result.ok, "precision overflow unexpectedly succeeded");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("32-bit precision")),
        "missing precision diagnostic: {:?}",
        result.errors
    );
}

#[test]
fn round_to_keeps_representable_precision_behavior() {
    let result = resilient::run_program("fn main() { println(round_to(3.14159, 2)); } main();");
    assert!(result.ok, "normal precision failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("3.14"),
        "unexpected output: {}",
        result.stdout
    );
}
