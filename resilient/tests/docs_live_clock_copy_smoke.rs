//! RES-3363: live-block docs describe no_std clock support boundaries plainly.

#[test]
fn live_block_docs_describe_no_std_clock_boundary_without_placeholder_wording() {
    let live_docs = include_str!("../../docs/live-block-semantics.md");
    let syntax = include_str!("../../SYNTAX.md");

    for expected in [
        "`no_std` runtime does not provide `within` wall-clock enforcement",
        "embedded targets ignore the clause until a monotonic clock hook is",
        "`no_std` embedded runtime does not enforce `within`",
        "wall-clock budgets yet; the check is currently std-only",
    ] {
        assert!(
            live_docs.contains(expected) || syntax.contains(expected),
            "live-block docs should plainly describe no_std clock support: {expected:?}"
        );
    }

    for stale in [
        "clock is a placeholder",
        "clock\nplaceholder",
        "real monotonic clock lands",
        "real monotonic\nclock is wired in",
    ] {
        assert!(
            !live_docs.contains(stale) && !syntax.contains(stale),
            "live-block docs should not retain placeholder clock wording: {stale:?}"
        );
    }
}
