use serde_json::Value;

fn json_file(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(repo_path(path)).expect("read json file"))
        .expect("parse json file")
}

fn repo_path(path: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join(path)
}

fn cargo_version() -> String {
    let cargo =
        std::fs::read_to_string(repo_path("resilient/Cargo.toml")).expect("read Cargo.toml");
    cargo
        .lines()
        .find_map(|line| line.strip_prefix("version = "))
        .and_then(|raw| raw.trim().trim_matches('"').split_whitespace().next())
        .expect("resilient/Cargo.toml version")
        .to_string()
}

#[test]
fn vscode_extension_version_matches_compiler_release() {
    let compiler = cargo_version();
    let package = json_file("vscode-extension/package.json");

    assert_eq!(package["version"], compiler);
}

#[test]
fn vscode_extension_publish_path_is_tag_and_pat_driven() {
    let package = json_file("vscode-extension/package.json");
    assert_eq!(package["publisher"], "fromamerica");
    assert_eq!(package["name"], "resilient-vscode");
    assert_eq!(
        package["scripts"]["vscode:publish"], "vsce publish",
        "package.json should keep the documented manual publish fallback"
    );

    let workflow = std::fs::read_to_string(repo_path(".github/workflows/vscode_extension.yml"))
        .expect("read workflow");
    assert!(
        workflow.contains("tags:\n      - \"v*\""),
        "vscode_extension.yml should publish from release tags"
    );
    assert!(
        workflow.contains("npx --yes @vscode/vsce publish --pat \"$VSCE_PAT\""),
        "workflow should publish with vsce and the PAT secret"
    );
    assert!(
        workflow.contains("VSCE_PAT: ${{ secrets.VSCE_PAT }}"),
        "workflow should read VSCE_PAT from GitHub Actions secrets"
    );

    let token_check = std::fs::read_to_string(repo_path(".github/workflows/vsce-token-check.yml"))
        .expect("read workflow");
    assert!(
        token_check.contains("verify-pat \"$PUBLISHER\" --pat \"$VSCE_PAT\""),
        "token check workflow should verify publisher access before release day"
    );
}
