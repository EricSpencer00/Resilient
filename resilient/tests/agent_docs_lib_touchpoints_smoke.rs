//! RES-3311: CLAUDE.md points core extension touchpoints at lib.rs.

#[test]
fn claude_agent_docs_use_lib_rs_for_core_extension_touchpoints() {
    let docs = include_str!("../../CLAUDE.md");

    for expected in [
        "older issues, handoffs, or scripts may say",
        "| Token enum variant | `lib.rs` `<EXTENSION_TOKENS>` |",
        "| Keyword \u{2192} Token mapping | `lib.rs` `<EXTENSION_KEYWORDS>` |",
        "| AST node variant | `lib.rs` `Node` enum",
        "### Minimal lib.rs touch example",
        "- Refactor `lib.rs` (the large core file)",
        "overlaps on `lib.rs` extension blocks are expected",
    ] {
        assert!(
            docs.contains(expected),
            "CLAUDE.md should point core extension guidance at lib.rs; missing {expected:?}"
        );
    }

    for stale in [
        "historical references in this file say",
        "| Token enum variant | `main.rs` `<EXTENSION_TOKENS>` |",
        "| Keyword \u{2192} Token mapping | `main.rs` `<EXTENSION_KEYWORDS>` |",
        "| AST node variant | `main.rs` `Node` enum",
        "### Minimal main.rs touch example",
        "- Refactor `main.rs` (35k+ lines)",
        "overlaps on `main.rs` extension blocks are expected",
    ] {
        assert!(
            !docs.contains(stale),
            "CLAUDE.md should not retain stale main.rs core guidance: {stale:?}"
        );
    }
}
