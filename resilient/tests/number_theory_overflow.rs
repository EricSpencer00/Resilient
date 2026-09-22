//! Boundary regressions for number-theory builtins.

use resilient::run_program;

#[test]
fn is_fibonacci_accepts_the_largest_i64_fibonacci() {
    let result = run_program("println(is_fibonacci(7540113804746346429));");

    assert!(result.ok, "errors: {:?}", result.errors);
    assert_eq!(result.stdout.trim(), "true");
}

#[test]
fn is_fibonacci_does_not_saturate_large_non_fibonacci_inputs() {
    let result = run_program("println(is_fibonacci(9223372036854775807));");

    assert!(result.ok, "errors: {:?}", result.errors);
    assert_eq!(result.stdout.trim(), "false");
}

#[test]
fn collatz_overflow_returns_a_typed_error() {
    let result = run_program("println(collatz_length(9223372036854775807));");

    assert!(!result.ok);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("3n + 1 overflowed")),
        "errors: {:?}",
        result.errors
    );
}
