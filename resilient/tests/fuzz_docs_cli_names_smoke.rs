//! RES-3281: fuzz docs and crash text use the shipped `rz` CLI name.

#[test]
fn fuzz_docs_and_targets_use_current_rz_cli_names() {
    let fuzz_readme = include_str!("../../fuzz/README.md");
    let parse_target = include_str!("../../fuzz/fuzz_targets/parse.rs");
    let lex_target = include_str!("../../fuzz/fuzz_targets/lex.rs");
    let jit_target = include_str!("../../fuzz/fuzz_targets/jit.rs");

    for expected in ["`rz -t`", "`rz --dump-tokens`", "`rz --jit`"] {
        assert!(
            fuzz_readme.contains(expected),
            "fuzz README target table should use current rz command {expected:?}"
        );
    }

    for expected in [
        "Same CLI-boundary pattern",
        "We shell out to `rz --jit <file>`",
        "avoiding private\n// in-process parser/lexer/JIT APIs",
    ] {
        assert!(
            jit_target.contains(expected),
            "JIT fuzz target should use current CLI-boundary rationale; missing {expected:?}"
        );
    }

    assert!(
        parse_target.contains("rz -t process crashed (signal) on fuzz input"),
        "parse fuzz crash message should name the rz command"
    );
    assert!(
        lex_target.contains("rz --dump-tokens process crashed (signal) on fuzz input"),
        "lex fuzz crash message should name the rz command"
    );

    for stale in [
        "`resilient -t`",
        "`resilient --dump-tokens`",
        "resilient process crashed",
        "resilient --dump-tokens process crashed",
        "binary-only",
        "no library\n// surface",
        "no library surface",
    ] {
        for (name, text) in [
            ("fuzz README", fuzz_readme),
            ("parse target", parse_target),
            ("lex target", lex_target),
            ("jit target", jit_target),
        ] {
            assert!(
                !text.contains(stale),
                "{name} should not retain stale fuzz CLI language: {stale:?}"
            );
        }
    }
}
