//! Regression coverage for overflow-safe number-theory divisor bounds.

#[test]
fn divisor_searches_use_non_overflowing_bounds() {
    let source = include_str!("../src/number_theory.rs");

    assert!(
        source.contains("while d <= n / d"),
        "prime factorization must not multiply its divisor bound"
    );
    assert!(
        source.contains("while p <= n / p"),
        "Euler totient must not multiply its divisor bound"
    );
    assert_eq!(
        source.matches("while i <= n / i").count(),
        2,
        "divisor enumeration and perfect-number checks must use division bounds"
    );
    assert!(!source.contains("while d * d <= n"));
    assert!(!source.contains("while p * p <= n"));
    assert!(!source.contains("while i * i <= n"));
}

#[test]
fn number_theory_results_remain_stable_for_regular_inputs() {
    let result = resilient::run_program(
        "println(prime_factors(12));\n\
         println(euler_totient(9));\n\
         println(divisors(12));\n\
         println(is_perfect(28));",
    );

    assert!(
        result.ok,
        "number-theory regression failed: {:?}",
        result.errors
    );
    assert!(result.stdout.contains("[2, 2, 3]"));
    assert!(result.stdout.contains("6"));
    assert!(result.stdout.contains("[1, 2, 3, 4, 6, 12]"));
    assert!(result.stdout.contains("true"));
}
