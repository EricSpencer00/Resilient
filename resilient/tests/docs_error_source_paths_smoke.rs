//! RES-3285: error docs point at library sources after the lib split.

#[test]
fn error_docs_use_current_parser_and_interpreter_source_paths() {
    let docs = [
        ("E0001", include_str!("../../docs/errors/E0001.md")),
        ("E0002", include_str!("../../docs/errors/E0002.md")),
        ("E0003", include_str!("../../docs/errors/E0003.md")),
        ("E0004", include_str!("../../docs/errors/E0004.md")),
        ("E0006", include_str!("../../docs/errors/E0006.md")),
        ("E0010", include_str!("../../docs/errors/E0010.md")),
    ];

    for (code, doc) in docs {
        assert!(
            doc.contains("resilient/src/lib.rs"),
            "{code} should point parser/interpreter source notes at resilient/src/lib.rs"
        );
        assert!(
            !doc.contains("resilient/src/main.rs"),
            "{code} should not point source notes at the CLI shim"
        );
    }

    let e0004 = include_str!("../../docs/errors/E0004.md");
    assert!(
        e0004.contains("resilient/src/typechecker.rs")
            && e0004.contains("resilient/src/compiler.rs"),
        "E0004 should preserve typechecker and compiler source references"
    );

    let e0006 = include_str!("../../docs/errors/E0006.md");
    assert!(
        e0006.contains("resilient/src/compiler.rs"),
        "E0006 should preserve the VM compiler source reference"
    );
}
