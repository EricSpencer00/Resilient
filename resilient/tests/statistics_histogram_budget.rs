//! RES-4762: resource-boundary coverage for `stats_histogram` bin allocation.
//!
//! The bin budget is the shared generated-collection limit
//! (`MAX_GENERATED_ELEMENTS`, 10_000_000). These tests pin the exact edge so
//! the accepted and rejected sides cannot drift apart silently.

const MAX_GENERATED_ELEMENTS: i64 = 10_000_000;

#[test]
fn histogram_accepts_bin_count_at_generated_budget() {
    let source = format!("println(len(stats_histogram([], {MAX_GENERATED_ELEMENTS})));\n");
    let result = resilient::run_program(&source);

    assert!(
        result.ok,
        "budget-sized histogram failed: {:?}",
        result.errors
    );
    assert!(
        result.stdout.contains("10000000"),
        "unexpected histogram length: {}",
        result.stdout
    );
}

#[test]
fn histogram_rejects_bin_count_one_past_generated_budget() {
    let source = format!("stats_histogram([], {});\n", MAX_GENERATED_ELEMENTS + 1);
    let result = resilient::run_program(&source);

    assert!(!result.ok, "bin count past the budget must be rejected");
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
fn histogram_budget_boundary_populated_counts_are_exact() {
    let source = format!(
        "let h = stats_histogram([0.0, 10.0], {MAX_GENERATED_ELEMENTS});\n\
         println(len(h));\n\
         println(h[0]);\n\
         println(h[{}]);\n",
        MAX_GENERATED_ELEMENTS - 1
    );
    let result = resilient::run_program(&source);

    assert!(
        result.ok,
        "populated budget histogram failed: {:?}",
        result.errors
    );
    let lines: Vec<&str> = result.stdout.trim().lines().collect();
    assert_eq!(lines, ["10000000", "1", "1"], "stdout: {}", result.stdout);
}

#[test]
fn histogram_rejects_non_positive_bin_counts() {
    for bins in ["0", "-1", "-9223372036854775808"] {
        let source = format!("stats_histogram([1.0, 2.0], {bins});\n");
        let result = resilient::run_program(&source);

        assert!(!result.ok, "bins={bins} must be rejected");
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("stats_histogram") && error.contains("must be > 0")),
            "bins={bins} unexpected diagnostics: {:?}",
            result.errors
        );
    }
}
