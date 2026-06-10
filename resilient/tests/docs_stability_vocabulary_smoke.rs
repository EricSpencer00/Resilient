//! RES-3154: keep public docs aligned with the CLI stability vocabulary.

fn public_docs() -> [(&'static str, &'static str); 4] {
    [
        ("README.md", include_str!("../../README.md")),
        ("docs/tooling.md", include_str!("../../docs/tooling.md")),
        (
            "docs/getting-started.md",
            include_str!("../../docs/getting-started.md"),
        ),
        (
            "docs/stable-regression-inventory.md",
            include_str!("../../docs/stable-regression-inventory.md"),
        ),
    ]
}

#[test]
fn docs_share_cli_status_vocabulary() {
    for (name, doc) in public_docs() {
        for expected in [
            "**Stable:** Supported for scripts and CI on the default build.",
            "**Backend-limited:** Stable when the named backend/build feature is present;",
            "unavailable builds print a rebuild hint.",
            "**Experimental:** User-facing, but policy/output may still evolve.",
        ] {
            assert!(
                doc.contains(expected),
                "{name} missing shared stability vocabulary {expected:?}"
            );
        }
    }
}

#[test]
fn backend_examples_name_required_features() {
    for (name, doc) in [
        ("README.md", include_str!("../../README.md")),
        ("docs/tooling.md", include_str!("../../docs/tooling.md")),
        (
            "docs/getting-started.md",
            include_str!("../../docs/getting-started.md"),
        ),
    ] {
        for (surface, feature) in [
            ("--jit", "--features jit"),
            ("--lsp", "--features lsp"),
            ("--emit-certificate", "--features z3"),
        ] {
            assert!(
                doc.contains(surface) && doc.contains(feature),
                "{name} should mention {feature} near backend-limited {surface} examples"
            );
        }
    }
}

#[test]
fn repl_launch_paths_are_consistent() {
    for (name, doc) in [
        ("README.md", include_str!("../../README.md")),
        ("docs/tooling.md", include_str!("../../docs/tooling.md")),
        (
            "docs/getting-started.md",
            include_str!("../../docs/getting-started.md"),
        ),
    ] {
        assert!(
            doc.contains("rz repl"),
            "{name} should mention the explicit `rz repl` alias"
        );
        assert!(
            doc.contains("bare `rz`") || doc.contains("no file argument"),
            "{name} should mention the bare `rz` REPL launch path"
        );
    }
}
