//! RES-3323: parser and extension comments point touchpoints at lib.rs.

#[test]
fn parser_extension_comments_use_lib_rs_touchpoints() {
    let files = [
        ("newtypes", include_str!("../src/newtypes.rs")),
        ("parser_recovery", include_str!("../src/parser_recovery.rs")),
        ("tuples", include_str!("../src/tuples.rs")),
        ("type_aliases", include_str!("../src/type_aliases.rs")),
        ("watch_mode", include_str!("../src/watch_mode.rs")),
    ];

    for (name, source) in files {
        assert!(
            !source.contains("main.rs"),
            "{name} should not point parser or extension touchpoints at main.rs"
        );
        assert!(
            !source.contains("main()"),
            "{name} should not describe watch/CLI dispatch as a direct main() hook"
        );
    }

    for expected in [
        "`<EXTENSION_PASSES>` block in `lib.rs`",
        "`Parser::synchronize_in_block` in `lib.rs`",
        "new `Node` variants and one new\n//! `Value` variant in `lib.rs`",
        "`Token::LeftParen` prefix arm in\n/// `lib.rs`",
        "**Token + keyword + AST node**: `lib.rs`",
        "**Parser**: `Parser::parse_type_alias` in `lib.rs`",
        "library dispatcher touchpoints in `lib.rs` are",
        "Entry point called from the library CLI dispatcher",
    ] {
        assert!(
            files.iter().any(|(_, source)| source.contains(expected)),
            "parser extension comments should include current lib.rs wording: {expected:?}"
        );
    }
}
