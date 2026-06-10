//! RES-3347: diagnostic comments should describe a shared model.

#[test]
fn diagnostic_comments_use_shared_model_wording() {
    let source = include_str!("../src/diag.rs");

    for expected in [
        "RES-119: shared Diagnostic data model.",
        "typed diagnostic data structures and\n// terminal renderer",
        "follow-up migrations can adopt these types phase by phase",
        "RES-119: Diagnostic model",
    ] {
        assert!(
            source.contains(expected),
            "diagnostic comments should describe the shared model: {expected:?}"
        );
    }

    for stale in [
        "RES-119 (scaffolding-only)",
        "Diagnostic scaffolding",
        "This section lands the data types",
    ] {
        assert!(
            !source.contains(stale),
            "diagnostic comments should not retain scaffolding wording: {stale:?}"
        );
    }
}
