//! Feature 41/50 — Package Manager.
//!
//! Extends `pkg_init` and `pkg_publish` with dependency resolution
//! against `resilient.toml`. The first slice ships:
//!
//! * Manifest parser: `name`, `version`, `[dependencies] foo = "1.2"`.
//! * Lock-file format: `resilient.lock` with resolved versions.
//! * Semver constraint matching (`^`, `~`, exact).

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub dependencies: HashMap<String, String>,
}

pub fn parse_manifest(s: &str) -> Manifest {
    let mut m = Manifest::default();
    let mut section = "package".to_string();
    for line in s.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(s) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = s.to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim().trim_matches('"');
            match section.as_str() {
                "package" => match k {
                    "name" => m.name = v.to_string(),
                    "version" => m.version = v.to_string(),
                    _ => {}
                },
                "dependencies" => {
                    m.dependencies.insert(k.to_string(), v.to_string());
                }
                _ => {}
            }
        }
    }
    m
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestSection {
    Package,
    Dependencies,
}

fn diagnostic(source_path: &str, line: usize, column: usize, message: &str) -> String {
    format!("{source_path}:{line}:{column}: error[pkg]: {message}")
}

fn first_non_ws_column(line: &str) -> usize {
    line.chars()
        .position(|ch| !ch.is_whitespace())
        .map(|idx| idx + 1)
        .unwrap_or(1)
}

fn parse_quoted_value(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if !raw.starts_with('"') {
        return None;
    }

    let mut escaped = false;
    let mut closing_quote = None;
    for (idx, ch) in raw.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '"' => {
                closing_quote = Some(idx);
                break;
            }
            _ => {}
        }
    }

    let closing_quote = closing_quote?;
    let value = &raw[1..closing_quote];
    let trailing = raw[closing_quote + 1..].trim();
    if trailing.is_empty() || trailing.starts_with('#') {
        Some(value.to_string())
    } else {
        None
    }
}

fn parse_manifest_strict(manifest: &str, manifest_display: &str) -> Result<Manifest, Vec<String>> {
    let mut out = Manifest::default();
    let mut errors = Vec::new();
    let mut section: Option<ManifestSection> = None;
    let mut saw_package = false;
    let mut saw_dependencies = false;
    let mut name_line: Option<(usize, usize)> = None;
    let mut version_line: Option<(usize, usize)> = None;
    let mut seen_dependencies = HashMap::new();

    for (idx, raw_line) in manifest.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with("[[") {
            errors.push(diagnostic(
                manifest_display,
                line_no,
                first_non_ws_column(raw_line),
                "invalid package manifest declaration shape: array-of-tables are not supported",
            ));
            section = None;
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix('[') {
            let header_col = first_non_ws_column(raw_line);
            let Some(end) = rest.find(']') else {
                errors.push(diagnostic(
                    manifest_display,
                    line_no,
                    header_col,
                    "invalid package manifest declaration shape: malformed section header",
                ));
                section = None;
                continue;
            };

            let header = rest[..end].trim();
            let trailing = rest[end + 1..].trim();
            if header.is_empty() || (!trailing.is_empty() && !trailing.starts_with('#')) {
                errors.push(diagnostic(
                    manifest_display,
                    line_no,
                    header_col,
                    "invalid package manifest declaration shape: malformed section header",
                ));
                section = None;
                continue;
            }

            match header {
                "package" => {
                    if saw_package {
                        errors.push(diagnostic(
                            manifest_display,
                            line_no,
                            header_col,
                            "invalid package manifest combination: duplicate `[package]` section",
                        ));
                        section = None;
                    } else {
                        saw_package = true;
                        section = Some(ManifestSection::Package);
                    }
                }
                "dependencies" => {
                    if saw_dependencies {
                        errors.push(diagnostic(
                            manifest_display,
                            line_no,
                            header_col,
                            "invalid package manifest combination: duplicate `[dependencies]` section",
                        ));
                        section = None;
                    } else {
                        saw_dependencies = true;
                        section = Some(ManifestSection::Dependencies);
                    }
                }
                other => {
                    errors.push(diagnostic(
                        manifest_display,
                        line_no,
                        header_col,
                        &format!(
                            "invalid package manifest declaration shape: unknown section `{other}`"
                        ),
                    ));
                    section = None;
                }
            }
            continue;
        }

        let Some(eq_idx) = raw_line.find('=') else {
            errors.push(diagnostic(
                manifest_display,
                line_no,
                first_non_ws_column(raw_line),
                "invalid package manifest declaration shape: expected `key = \"value\"`",
            ));
            continue;
        };

        let key = raw_line[..eq_idx].trim();
        let value_part = raw_line[eq_idx + 1..].trim();
        let key_col = raw_line
            .find(key)
            .map(|idx| idx + 1)
            .unwrap_or_else(|| first_non_ws_column(raw_line));

        if key.is_empty() {
            errors.push(diagnostic(
                manifest_display,
                line_no,
                first_non_ws_column(raw_line),
                "invalid package manifest declaration shape: missing key",
            ));
            continue;
        }

        let Some(value) = parse_quoted_value(value_part) else {
            errors.push(diagnostic(
                manifest_display,
                line_no,
                key_col,
                "invalid package manifest declaration shape: expected quoted string value",
            ));
            continue;
        };

        match section {
            Some(ManifestSection::Package) => match key {
                "name" => {
                    if name_line.is_some() {
                        errors.push(diagnostic(
                            manifest_display,
                            line_no,
                            key_col,
                            "invalid package manifest combination: duplicate `name` declaration",
                        ));
                    } else {
                        name_line = Some((line_no, key_col));
                        out.name = value;
                    }
                }
                "version" => {
                    if version_line.is_some() {
                        errors.push(diagnostic(
                            manifest_display,
                            line_no,
                            key_col,
                            "invalid package manifest combination: duplicate `version` declaration",
                        ));
                    } else {
                        version_line = Some((line_no, key_col));
                        out.version = value;
                    }
                }
                other => {
                    errors.push(diagnostic(
                        manifest_display,
                        line_no,
                        key_col,
                        &format!(
                            "invalid package manifest combination: `{other}` is not allowed in `[package]`"
                        ),
                    ));
                }
            },
            Some(ManifestSection::Dependencies) => {
                if seen_dependencies.contains_key(key) {
                    errors.push(diagnostic(
                        manifest_display,
                        line_no,
                        key_col,
                        &format!(
                            "invalid package manifest combination: duplicate dependency `{key}`"
                        ),
                    ));
                } else {
                    seen_dependencies.insert(key.to_string(), (line_no, key_col));
                    out.dependencies.insert(key.to_string(), value);
                }
            }
            None => {
                errors.push(diagnostic(
                    manifest_display,
                    line_no,
                    key_col,
                    "invalid package manifest combination: declaration appears outside `[package]` or `[dependencies]`",
                ));
            }
        }
    }

    if out.name.is_empty() {
        let (line, column) = name_line.or(version_line).unwrap_or((1, 1));
        errors.push(diagnostic(
            manifest_display,
            line,
            column,
            "manifest missing required `name` field in `[package]` section",
        ));
    }

    if out.version.is_empty() {
        let (line, column) = version_line.or(name_line).unwrap_or((1, 1));
        errors.push(diagnostic(
            manifest_display,
            line,
            column,
            "manifest missing required `version` field in `[package]` section",
        ));
    } else if !is_valid_semver(&out.version) {
        let (line, column) = version_line.unwrap_or((1, 1));
        errors.push(diagnostic(
            manifest_display,
            line,
            column,
            &format!(
                "manifest `version` `{}` is not a valid semver triple (MAJOR.MINOR.PATCH)",
                out.version
            ),
        ));
    }

    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemverRange {
    Exact,
    Caret,
    Tilde,
}

pub fn parse_constraint(s: &str) -> (SemverRange, String) {
    if let Some(rest) = s.strip_prefix('^') {
        (SemverRange::Caret, rest.to_string())
    } else if let Some(rest) = s.strip_prefix('~') {
        (SemverRange::Tilde, rest.to_string())
    } else {
        (SemverRange::Exact, s.to_string())
    }
}

pub fn matches(constraint: &str, version: &str) -> bool {
    let (kind, base) = parse_constraint(constraint);
    let parse = |s: &str| -> Option<(u64, u64, u64)> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        Some((
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        ))
    };
    let (a_maj, a_min, a_pat) = match parse(&base) {
        Some(v) => v,
        None => return false,
    };
    let (b_maj, b_min, b_pat) = match parse(version) {
        Some(v) => v,
        None => return false,
    };
    match kind {
        SemverRange::Exact => (a_maj, a_min, a_pat) == (b_maj, b_min, b_pat),
        SemverRange::Caret => b_maj == a_maj && (b_min, b_pat) >= (a_min, a_pat),
        SemverRange::Tilde => b_maj == a_maj && b_min == a_min && b_pat >= a_pat,
    }
}

/// Validate the project's `rz.toml` / `resilient.toml` manifest if one
/// exists adjacent to `source_path`.
///
/// Checks:
/// 1. Manifest declaration shapes and section/key combinations are valid.
/// 2. Package section includes required `name` and `version` fields.
/// 3. Version field valid semver triple (`MAJOR.MINOR.PATCH`).
/// 4. Dependency constraint parseable (`^`, `~`, exact).
/// 5. `resilient.lock` exists, locked versions satisfy constraints.
pub(crate) fn check(_program: &Node, source_path: &str) -> Result<(), String> {
    let source_dir = std::path::Path::new(source_path)
        .parent()
        .unwrap_or(std::path::Path::new("."));

    let manifest_path = ["rz.toml", "resilient.toml"]
        .iter()
        .map(|name| source_dir.join(name))
        .find(|p| p.exists());

    let manifest_path = match manifest_path {
        Some(path) => path,
        None => return Ok(()),
    };

    let manifest_display = manifest_path.display().to_string();
    let manifest_content = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };

    let manifest = match parse_manifest_strict(&manifest_content, &manifest_display) {
        Ok(manifest) => manifest,
        Err(errors) => {
            return Err(errors.join(
                "
",
            ));
        }
    };

    let mut errors: Vec<String> = Vec::new();

    for (dep, constraint) in &manifest.dependencies {
        let (_, base) = parse_constraint(constraint);
        if !is_valid_semver(&base) {
            errors.push(format!(
                "{manifest_display}:0:0: error[pkg]: dependency `{dep}` has                  invalid constraint `{constraint}`; expected semver                  optionally prefixed by `^` or `~`"
            ));
        }
    }

    let lock_path = source_dir.join("resilient.lock");
    if lock_path.exists() {
        if let Ok(lock_content) = std::fs::read_to_string(&lock_path) {
            let locked = parse_lock_file(&lock_content);
            for (dep, constraint) in &manifest.dependencies {
                if let Some(locked_version) = locked.get(dep) {
                    if !matches(constraint, locked_version) {
                        errors.push(format!(
                            "{manifest_display}:0:0: error[pkg]: locked version                              `{locked_version}` for `{dep}` does not satisfy                              constraint `{constraint}`"
                        ));
                    }
                }
            }
        }
    }

    if errors.is_empty() {
        if !manifest.dependencies.is_empty() {
            eprintln!(
                "pkg: manifest `{}` v{} with {} dependency/ies validated",
                manifest.name,
                manifest.version,
                manifest.dependencies.len()
            );
        }
        Ok(())
    } else {
        Err(errors.join(
            "
",
        ))
    }
}

fn is_valid_semver(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| p.parse::<u64>().is_ok())
}

/// Parse a simple `resilient.lock` format:
/// ```text
/// dep_name = "resolved_version"
/// ```
fn parse_lock_file(s: &str) -> std::collections::HashMap<String, String> {
    let mut locked = std::collections::HashMap::new();
    for line in s.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim().to_string();
            let v = v.trim().trim_matches('"').to_string();
            locked.insert(k, v);
        }
    }
    locked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parses() {
        let s = r#"
[package]
name = "myapp"
version = "1.0.0"

[dependencies]
foo = "^1.2.0"
bar = "~0.3.1"
"#;
        let m = parse_manifest(s);
        assert_eq!(m.name, "myapp");
        assert_eq!(
            m.dependencies.get("foo").map(|s| s.as_str()),
            Some("^1.2.0")
        );
    }

    #[test]
    fn caret_matching() {
        assert!(matches("^1.2.3", "1.2.3"));
        assert!(matches("^1.2.3", "1.5.0"));
        assert!(!matches("^1.2.3", "2.0.0"));
        assert!(!matches("^1.2.3", "1.2.2"));
    }

    #[test]
    fn tilde_matching() {
        assert!(matches("~1.2.3", "1.2.3"));
        assert!(matches("~1.2.3", "1.2.5"));
        assert!(!matches("~1.2.3", "1.3.0"));
    }

    #[test]
    fn exact_matching() {
        assert!(matches("1.2.3", "1.2.3"));
        assert!(!matches("1.2.3", "1.2.4"));
    }

    // ── is_valid_semver ──────────────────────────────────────────────────────

    #[test]
    fn valid_semver_accepted() {
        assert!(is_valid_semver("1.2.3"));
        assert!(is_valid_semver("0.0.0"));
        assert!(is_valid_semver("100.200.300"));
    }

    #[test]
    fn invalid_semver_rejected() {
        assert!(!is_valid_semver("1.2"));
        assert!(!is_valid_semver("1.2.3.4"));
        assert!(!is_valid_semver("1.2.x"));
        assert!(!is_valid_semver(""));
    }

    // ── parse_lock_file ──────────────────────────────────────────────────────

    #[test]
    fn lock_file_parses_entries() {
        let s = r#"
# Generated lock file
foo = "1.2.3"
bar = "0.3.5"
"#;
        let locked = parse_lock_file(s);
        assert_eq!(locked.get("foo").map(|s| s.as_str()), Some("1.2.3"));
        assert_eq!(locked.get("bar").map(|s| s.as_str()), Some("0.3.5"));
    }

    // ── check() ──────────────────────────────────────────────────────────────

    #[test]
    fn check_ok_when_no_manifest_in_tmpdir() {
        let tmp = std::env::temp_dir().join("__resilient_check_no_manifest_test.rz");
        std::fs::write(&tmp, b"fn f() {}").unwrap();
        let (prog, _) = crate::parse("fn f() {}");
        let result = check(&prog, tmp.to_str().unwrap());
        assert!(result.is_ok());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn check_validates_manifest_with_valid_entries() {
        let dir = std::env::temp_dir().join("__resilient_pkg_valid");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "1.0.0"
[dependencies]
utils = "^2.0.0"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        assert!(result.is_ok(), "expected ok, got: {:?}", result);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_errors_on_invalid_version() {
        let dir = std::env::temp_dir().join("__resilient_pkg_badver");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "not-semver"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        assert!(result.is_err(), "expected error for invalid version");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_errors_on_invalid_dep_constraint() {
        let dir = std::env::temp_dir().join("__resilient_pkg_baddep");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "1.0.0"
[dependencies]
utils = "latest"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        assert!(result.is_err(), "expected error for invalid constraint");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_rejects_malformed_section_header() {
        let dir = std::env::temp_dir().join("__resilient_pkg_badheader");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package
name = "myapp"
version = "1.0.0"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap()).unwrap_err();
        assert!(
            result.contains("rz.toml:2:1: error[pkg]: invalid package manifest declaration shape: malformed section header"),
            "unexpected diagnostic: {result}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_rejects_malformed_entry_shape() {
        let dir = std::env::temp_dir().join("__resilient_pkg_badentry");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = myapp
version = "1.0.0"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap()).unwrap_err();
        assert!(
            result.contains("rz.toml:3:1: error[pkg]: invalid package manifest declaration shape: expected quoted string value"),
            "unexpected diagnostic: {result}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_rejects_invalid_package_combination() {
        let dir = std::env::temp_dir().join("__resilient_pkg_badcombo");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "1.0.0"
extra = "nope"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap()).unwrap_err();
        assert!(
            result.contains("rz.toml:5:1: error[pkg]: invalid package manifest combination: `extra` is not allowed in `[package]`"),
            "unexpected diagnostic: {result}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_rejects_missing_required_fields() {
        let dir = std::env::temp_dir().join("__resilient_pkg_missing_version");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap()).unwrap_err();
        assert!(
            result.contains("rz.toml:3:1: error[pkg]: manifest missing required `version` field in `[package]` section"),
            "unexpected diagnostic: {result}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
