//! RES-3337: VS Code README avoids internal roadmap tracking wording.

#[test]
fn vscode_readme_roadmap_note_uses_public_wording() {
    let readme = include_str!("../../vscode-extension/README.md");

    assert!(
        readme.contains("V2 design work is tracked separately and will be scoped independently"),
        "VS Code README should describe V2 work with public roadmap wording"
    );
    assert!(
        readme.contains("This release is purely additive on the V1 surface."),
        "VS Code README should preserve the V1 additive-surface note"
    );
    assert!(
        !readme.contains("tracked internally"),
        "VS Code README should not expose internal tracking language"
    );
}
