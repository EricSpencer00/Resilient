//! RES-3335: safety docs use direct support-boundary wording.

#[test]
fn safety_docs_avoid_internal_future_scaffold_wording() {
    let concurrency = include_str!("../../docs/concurrency.md");
    let do178c = include_str!("../../docs/standards/do-178c.md");

    for expected in [
        "Real-time scheduling guarantees depend on the planned AOT path;",
        "today's runtime does not provide them, so hard-RT code stays in C.",
        "**Robustness testing support.** Contracts that *cannot*",
        "instrumented oracles for robustness testing",
    ] {
        assert!(
            concurrency.contains(expected) || do178c.contains(expected),
            "safety docs should use direct support-boundary wording: {expected:?}"
        );
    }

    for stale in [
        "future work item gated\n  on AOT",
        "**Robustness testing scaffolding.**",
    ] {
        assert!(
            !concurrency.contains(stale) && !do178c.contains(stale),
            "safety docs should not retain internal roadmap/scaffold wording: {stale:?}"
        );
    }
}
