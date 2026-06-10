//! RES-3307: VS Code README command examples match the `rz` extension defaults.

#[test]
fn vscode_readme_uses_rz_binary_names_consistently() {
    let readme = include_str!("../../vscode-extension/README.md");
    let package_json = include_str!("../../vscode-extension/package.json");
    let extension = include_str!("../../vscode-extension/src/extension.ts");

    for expected in [
        "it contains the rz binary",
        "rz --typecheck --audit divide.rz",
        "| `resilient.serverPath` | `rz` | Path to the `rz` binary.",
        "resilient/target/debug/rz",
    ] {
        assert!(
            readme.contains(expected),
            "VS Code README should use current rz binary wording; missing {expected:?}"
        );
    }

    assert!(
        package_json.contains("\"resilient.serverPath\"")
            && package_json.contains("\"default\": \"rz\""),
        "VS Code package configuration should default resilient.serverPath to rz"
    );
    assert!(
        extension.contains("serverPath\", \"rz\""),
        "VS Code extension runtime fallback should default serverPath to rz"
    );

    for stale in [
        "resilient --typecheck --audit divide.rz",
        "| `resilient.serverPath` | `resilient` |",
        "Path to the `resilient` binary.",
    ] {
        assert!(
            !readme.contains(stale),
            "VS Code README should not retain stale resilient-binary wording: {stale:?}"
        );
    }
}
