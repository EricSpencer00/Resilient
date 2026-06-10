//! RES-3279: fuzz docs know the compiler crate has a library target.

#[test]
fn fuzz_docs_use_current_library_and_cli_language() {
    let fuzz_readme = include_str!("../../fuzz/README.md");
    let fuzz_manifest = include_str!("../../fuzz/Cargo.toml");
    let parse_target = include_str!("../../fuzz/fuzz_targets/parse.rs");
    let lex_target = include_str!("../../fuzz/fuzz_targets/lex.rs");
    let compiler_manifest = include_str!("../Cargo.toml");

    assert!(
        compiler_manifest.contains("[lib]\nname = \"resilient\"\npath = \"src/lib.rs\""),
        "compiler manifest should expose the library target this docs smoke test relies on"
    );

    for expected in [
        "The compiler crate now exposes a library target",
        "targets still exercise the shipped CLI boundary",
        "committing a\nsmall public fuzzing API",
        "rz -t fuzz/artifacts/parse/crash-<hash>",
        "`resilient/src/lib.rs` or an integration test",
    ] {
        assert!(
            fuzz_readme.contains(expected),
            "fuzz README should use current library/CLI language; missing {expected:?}"
        );
    }

    for expected in [
        "All targets shell out to the built `rz` binary.",
        "The compiler has\n# a library target",
        "shipped CLI boundary",
    ] {
        assert!(
            fuzz_manifest.contains(expected),
            "fuzz manifest comments should use current subprocess rationale; missing {expected:?}"
        );
    }

    for expected in [
        "Spawn `rz -t --seed 0 <tempfile>`",
        "These fuzzers exercise the\n// shipped CLI boundary",
        "parser internals are not a\n// committed public fuzzing API",
        "The `rz` binary must be built",
    ] {
        assert!(
            parse_target.contains(expected),
            "parse fuzz target comments should use current subprocess rationale; missing {expected:?}"
        );
    }

    for expected in [
        "Same CLI-boundary pattern",
        "built `rz` binary with `--dump-tokens`",
        "lexer internals are not a committed\n// public fuzzing API",
    ] {
        assert!(
            lex_target.contains(expected),
            "lex fuzz target comments should use current subprocess rationale; missing {expected:?}"
        );
    }

    for stale in [
        "binary-only",
        "no `src/lib.rs`",
        "no lib surface to link",
        "resilient -t fuzz/artifacts/parse/crash-<hash>",
        "`resilient/src/main.rs` `mod tests`",
    ] {
        for (name, text) in [
            ("fuzz README", fuzz_readme),
            ("fuzz manifest", fuzz_manifest),
            ("parse target", parse_target),
            ("lex target", lex_target),
        ] {
            assert!(
                !text.contains(stale),
                "{name} should not retain stale fuzz language: {stale:?}"
            );
        }
    }
}
