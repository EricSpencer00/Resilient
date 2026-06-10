//! RES-3299: agent docs use current core compiler file paths.

#[test]
fn claude_doc_core_file_examples_use_lib_rs() {
    let docs = include_str!("../../CLAUDE.md");

    for expected in [
        "agent-scripts/check-overlaps.sh resilient/src/lib.rs resilient/src/typechecker.rs resilient/src/lexer_logos.rs",
        "agent-scripts/claim-files.sh res-NNN-short-title resilient/src/lib.rs resilient/src/typechecker.rs resilient/src/lexer_logos.rs",
    ] {
        assert!(
            docs.contains(expected),
            "CLAUDE.md should use current core-file examples; missing {expected:?}"
        );
    }

    for stale in [
        "check-overlaps.sh resilient/src/main.rs",
        "claim-files.sh res-NNN-short-title resilient/src/main.rs",
    ] {
        assert!(
            !docs.contains(stale),
            "CLAUDE.md should not send agents to the CLI shim in core-file examples"
        );
    }
}
