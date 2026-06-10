//! RES-3377: root syntax docs describe unsupported control/pattern forms plainly.

#[test]
fn root_syntax_docs_describe_loop_and_range_binding_boundaries() {
    let syntax = include_str!("../../SYNTAX.md");

    for expected in [
        "loop-with-value forms such as `let x = loop { break v; }` are not\nsupported yet.",
        "Range patterns bind no names today; forms such as\n`1..=5 @ x` are not supported yet.",
    ] {
        assert!(
            syntax.contains(expected),
            "root syntax docs should state unsupported syntax boundaries: {expected:?}"
        );
    }

    for stale in [
        "loop { break v; }` (loop-with-value) is a future enhancement.",
        "`1..=5 @ x` binding\nis a future enhancement).",
    ] {
        assert!(
            !syntax.contains(stale),
            "root syntax docs should not use vague future-enhancement wording: {stale:?}"
        );
    }
}
