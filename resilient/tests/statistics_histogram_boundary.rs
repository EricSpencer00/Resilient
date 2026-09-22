//! Regression coverage for the bounded `stats_histogram` allocation path.

#[test]
fn histogram_rejects_an_unbounded_bin_count_before_allocation() {
    let result = resilient::run_program("stats_histogram([], 9223372036854775807);\n");

    assert!(!result.ok, "an unbounded bin count must be rejected");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("stats_histogram") && error.contains("too large")),
        "unexpected diagnostics: {:?}",
        result.errors
    );
}

#[test]
fn histogram_preserves_bounded_counts() {
    let result = resilient::run_program("println(stats_histogram([0.0, 1.0, 2.0], 3));\n");

    assert!(result.ok, "bounded histogram failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[1, 1, 1]"),
        "unexpected histogram output: {}",
        result.stdout
    );
}
