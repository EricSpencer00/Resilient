//! Resource bounds for collection-producing data utilities.

fn assert_expansion_rejected(source: &str, builtin: &str) {
    let result = resilient::run_program(source);
    assert!(!result.ok, "{builtin} accepted an oversized expansion");
    let diagnostics = result.errors.join("\n");
    assert!(
        diagnostics.contains("result would exceed 10000000 elements"),
        "{builtin} returned an unexpected diagnostic: {diagnostics}"
    );
}

#[test]
fn linspace_rejects_oversized_count_before_allocation() {
    assert_expansion_rejected("linspace(0.0, 1.0, 10000001);", "linspace");
}

#[test]
fn logspace_rejects_oversized_count_before_allocation() {
    assert_expansion_rejected("logspace(0.0, 1.0, 10000001);", "logspace");
}

#[test]
fn rle_decode_rejects_oversized_run_before_expansion() {
    assert_expansion_rejected("rle_decode([[10000001, 0]]);", "rle_decode");
}
