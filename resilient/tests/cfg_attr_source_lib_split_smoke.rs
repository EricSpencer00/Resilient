//! RES-3315: cfg_attr source comments point core hooks at lib.rs.

#[test]
fn cfg_attr_comments_use_lib_rs_for_core_hooks() {
    let source = include_str!("../src/cfg_attr.rs");

    for expected in [
        "`lib.rs` `<EXTENSION_TOKENS>`",
        "`lib.rs` `parse_statement`",
        "the `lib.rs` extension footprint minimal",
        "the driver in `lib.rs` after CLI parsing",
        "The parser hook in\n/// `lib.rs` consults `is_active`",
        "The body of the cfg attribute lives here so `lib.rs` only adds a single",
        "in `lib.rs`; the dispatch hook lives",
    ] {
        assert!(
            source.contains(expected),
            "cfg_attr comments should point core hooks at lib.rs; missing {expected:?}"
        );
    }

    for stale in [
        "`main.rs` `<EXTENSION_TOKENS>`",
        "`main.rs` `parse_statement`",
        "the `main.rs` extension footprint minimal",
        "the driver in `main.rs` after CLI parsing",
        "The parser hook in\n/// `main.rs` consults `is_active`",
        "The body of the cfg attribute lives here so `main.rs` only adds a single",
        "in `main.rs`; the dispatch hook lives",
    ] {
        assert!(
            !source.contains(stale),
            "cfg_attr comments should not retain stale main.rs hook wording: {stale:?}"
        );
    }
}
