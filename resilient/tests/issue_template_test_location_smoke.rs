//! RES-3297: good-first-issue template uses current test locations.

#[test]
fn good_first_issue_template_uses_current_test_guidance() {
    let template = include_str!("../../.github/ISSUE_TEMPLATE/good_first_issue.md");

    for expected in [
        "`foo.rz` example runs",
        "a unit test in `resilient/src/lib.rs`",
        "focused integration test under `resilient/tests/`",
        "`resilient/examples/foo.rz` with a `foo.expected.txt` sidecar",
    ] {
        assert!(
            template.contains(expected),
            "good-first-issue template should use current contributor guidance; missing {expected:?}"
        );
    }

    for stale in [
        "`foo.rs` example runs",
        "resilient/src/main.rs",
        "mod tests`, or a new `resilient/examples/foo.rs`",
    ] {
        assert!(
            !template.contains(stale),
            "good-first-issue template should not retain stale guidance: {stale:?}"
        );
    }
}
