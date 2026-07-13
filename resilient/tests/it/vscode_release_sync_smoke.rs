//! RES-3783: VS Code extension release metadata stays coherent.
//!
//! `vscode-extension/package.json` is published to the Marketplace by
//! `.github/workflows/vscode_extension.yml` on a `v*` tag push. These
//! checks guard the invariants that workflow (and
//! `docs/VSCODE_EXTENSION_RELEASE.md`) assume hold:
//!   - the publisher/name identify the extension we actually own
//!   - the version is well-formed semver
//!   - the version tracks `resilient/Cargo.toml` (see the doc for the
//!     Marketplace 1.5.3-vs-0.2.x divergence this does NOT cover — that
//!     is a separate, human-driven reconciliation, not a test).

use serde_json::Value;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("resilient crate has repo parent")
        .to_path_buf()
}

fn package_json() -> Value {
    let path = repo_root().join("vscode-extension/package.json");
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn cargo_version() -> String {
    let path = repo_root().join("resilient/Cargo.toml");
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    raw.lines()
        .find_map(|line| line.strip_prefix("version = "))
        .and_then(|raw| raw.trim().trim_matches('"').split_whitespace().next())
        .unwrap_or_else(|| panic!("no `version = \"...\"` line in {}", path.display()))
        .to_string()
}

/// Parses a `major.minor.patch` (optionally with a `-pre`/`+build`
/// suffix) string into its numeric components. Hand-rolled rather than
/// pulling in a `semver` dependency, matching this crate's existing
/// "no new dependency for a one-off check" convention.
fn parse_semver_core(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

#[test]
fn vscode_package_identifies_the_owned_extension() {
    let package = package_json();
    assert_eq!(
        package["publisher"], "fromamerica",
        "vscode-extension/package.json publisher must match the Marketplace publisher \
         the VSCE_PAT secret is scoped to, or `vsce publish` will fail"
    );
    assert_eq!(
        package["name"], "resilient-vscode",
        "vscode-extension/package.json name must match the published extension id"
    );
}

#[test]
fn vscode_package_version_is_valid_semver() {
    let package = package_json();
    let version = package["version"]
        .as_str()
        .expect("package.json version must be a string");
    assert!(
        !version.is_empty(),
        "package.json version must not be empty"
    );
    assert!(
        parse_semver_core(version).is_some(),
        "package.json version {version:?} must be parseable major.minor.patch semver"
    );
}

#[test]
fn vscode_package_version_tracks_compiler_release() {
    let package = package_json();
    let extension_version = package["version"]
        .as_str()
        .expect("package.json version must be a string");
    let compiler_version = cargo_version();

    assert_eq!(
        extension_version, compiler_version,
        "vscode-extension/package.json version should track resilient/Cargo.toml \
         (see docs/VSCODE_EXTENSION_RELEASE.md for the current Marketplace divergence \
         and the reconciliation options — do not roll either version backward to \
         force this assertion to pass)"
    );
}

#[test]
fn vscode_extension_workflow_publishes_on_tag_with_pat() {
    let workflow =
        std::fs::read_to_string(repo_root().join(".github/workflows/vscode_extension.yml"))
            .expect("read vscode_extension.yml");

    assert!(
        workflow.contains("tags:") && workflow.contains("\"v*\""),
        "vscode_extension.yml should trigger packaging/publish from v* tags"
    );
    assert!(
        workflow.contains("npx --yes @vscode/vsce publish --pat \"$VSCE_PAT\""),
        "vscode_extension.yml should publish via vsce using the VSCE_PAT secret"
    );
    assert!(
        workflow.contains("VSCE_PAT: ${{ secrets.VSCE_PAT }}"),
        "vscode_extension.yml should read VSCE_PAT from GitHub Actions secrets, not a literal"
    );
}
