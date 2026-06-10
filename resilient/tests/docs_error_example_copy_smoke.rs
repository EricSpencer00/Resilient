//! RES-3235: error docs use Resilient snippets and `.rz` diagnostics.

const ERROR_DOCS: [(&str, &str); 10] = [
    ("E0001", include_str!("../../docs/errors/E0001.md")),
    ("E0002", include_str!("../../docs/errors/E0002.md")),
    ("E0003", include_str!("../../docs/errors/E0003.md")),
    ("E0004", include_str!("../../docs/errors/E0004.md")),
    ("E0005", include_str!("../../docs/errors/E0005.md")),
    ("E0006", include_str!("../../docs/errors/E0006.md")),
    ("E0007", include_str!("../../docs/errors/E0007.md")),
    ("E0008", include_str!("../../docs/errors/E0008.md")),
    ("E0009", include_str!("../../docs/errors/E0009.md")),
    ("E0010", include_str!("../../docs/errors/E0010.md")),
];

#[test]
fn error_docs_use_resilient_fences_and_rz_filenames() {
    for (code, doc) in ERROR_DOCS {
        assert!(
            doc.contains("```resilient"),
            "{code} should mark language snippets as resilient"
        );
        assert!(
            !doc.contains("```rs"),
            "{code} should not use Rust-flavored fences for Resilient snippets"
        );
        assert!(
            doc.contains("scratch.rz:"),
            "{code} should report diagnostics against a .rz file"
        );
        assert!(
            !doc.contains("scratch.rs"),
            "{code} should not report diagnostics against a .rs file"
        );
    }
}
