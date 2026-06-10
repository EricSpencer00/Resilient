//! RES-3241: language reference import examples use `.rz` paths.

#[test]
fn language_reference_use_semantics_use_rz_paths() {
    let doc = include_str!("../../docs/language-reference.md");

    assert!(
        doc.contains("`use \"path/to/file.rz\";` is a textual splice"),
        "language reference should show a .rz import path"
    );
    assert!(
        !doc.contains("path/to/file.rs"),
        "language reference should not show a Rust file extension for imports"
    );
}
