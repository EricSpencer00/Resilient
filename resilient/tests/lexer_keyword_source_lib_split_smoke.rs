//! RES-3319: lexer and REPL keyword comments point at lib.rs.

#[test]
fn lexer_keyword_comments_use_lib_rs_sources() {
    let lexer_logos = include_str!("../src/lexer_logos.rs");
    let repl = include_str!("../src/repl.rs");

    assert!(
        lexer_logos.contains("hand-rolled scanner in `lib.rs` produces"),
        "lexer_logos should point scanner parity comments at lib.rs"
    );
    assert!(
        !lexer_logos.contains("hand-rolled scanner in `main.rs` produces"),
        "lexer_logos should not retain stale main.rs scanner wording"
    );

    assert!(
        repl.contains("keyword table in `lib.rs::Lexer::next_token`"),
        "REPL keyword comments should point at lib.rs::Lexer::next_token"
    );
    assert!(
        !repl.contains("keyword table in `main.rs::Lexer::next_token`"),
        "REPL keyword comments should not retain stale main.rs keyword wording"
    );
}
