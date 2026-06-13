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
/// 1. Manifest parses without errors (name, version present).
/// 2. Version field is a valid semver triple (`MAJOR.MINOR.PATCH`).
/// 3. Each dependency constraint is parseable (`^`, `~`, or exact).
/// 4. If a `resilient.lock` exists, locked versions satisfy the constraints.
pub(crate) fn check(_program: &Node, source_path: &str) -> Result<(), String> {
    let source_dir = std::path::Path::new(source_path)
        .parent()
        .unwrap_or(std::path::Path::new("."));

    // Look for manifest in the source directory or cwd
    let manifest_path = ["rz.toml", "resilient.toml"]
        .iter()
        .map(|name| source_dir.join(name))
        .find(|p| p.exists());

    let manifest_content = match manifest_path {
        Some(ref p) => match std::fs::read_to_string(p) {
            Ok(s) => s,
            Err(_) => return Ok(()), // unreadable — skip silently
        },
        None => return Ok(()), // no manifest — nothing to check
    };

    let manifest = parse_manifest(&manifest_content);

    let mut errors: Vec<String> = Vec::new();

    // Validate package name and version
    if manifest.name.is_empty() {
        errors.push(format!(
            "{source_path}:0:0: error[pkg]: manifest is missing `name` field"
        ));
    }
    if manifest.version.is_empty() {
        errors.push(format!(
            "{source_path}:0:0: error[pkg]: manifest is missing `version` field"
        ));
    } else if !is_valid_semver(&manifest.version) {
        errors.push(format!(
            "{source_path}:0:0: error[pkg]: manifest `version` `{}` is not a \
             valid semver triple (MAJOR.MINOR.PATCH)",
            manifest.version
        ));
    }

    // Validate dependency constraint syntax
    for (dep, constraint) in &manifest.dependencies {
        let (_, base) = parse_constraint(constraint);
        if !is_valid_semver(&base) {
            errors.push(format!(
                "{source_path}:0:0: error[pkg]: dependency `{dep}` has \
                 invalid constraint `{constraint}` — expected semver \
                 optionally prefixed with `^` or `~`"
            ));
        }
    }

    // If lock file exists, check locked versions satisfy constraints
    let lock_path = source_dir.join("resilient.lock");
    if lock_path.exists() {
        if let Ok(lock_content) = std::fs::read_to_string(&lock_path) {
            let locked = parse_lock_file(&lock_content);
            for (dep, constraint) in &manifest.dependencies {
                if let Some(locked_version) = locked.get(dep) {
                    if !matches(constraint, locked_version) {
                        errors.push(format!(
                            "{source_path}:0:0: error[pkg]: locked version \
                             `{locked_version}` for `{dep}` does not satisfy \
                             constraint `{constraint}`"
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
        Err(errors.join("\n"))
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

    // ── Malformed-input regression corpus ──────────────────────────────────
    // Comprehensive test cases for malformed package_manager declarations,
    // invalid manifests, and constraint violations.

    #[test]
    fn check_errors_on_missing_name_field() {
        let dir = std::env::temp_dir().join("__resilient_pkg_noname");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
version = "1.0.0"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("missing name must fail");
        assert!(err.contains("missing `name` field"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_errors_on_missing_version_field() {
        let dir = std::env::temp_dir().join("__resilient_pkg_noversion");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("missing version must fail");
        assert!(err.contains("missing `version` field"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_errors_on_version_with_trailing_part() {
        let dir = std::env::temp_dir().join("__resilient_pkg_badver_extra");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "1.0.0.1"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("too many version parts must fail");
        assert!(err.contains("not a valid semver triple"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_errors_on_version_with_missing_part() {
        let dir = std::env::temp_dir().join("__resilient_pkg_badver_missing");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "1.0"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("too few version parts must fail");
        assert!(err.contains("not a valid semver triple"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_errors_on_version_with_nonnumeric() {
        let dir = std::env::temp_dir().join("__resilient_pkg_badver_nonnumeric");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "1.x.3"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("non-numeric version must fail");
        assert!(err.contains("not a valid semver triple"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_errors_on_constraint_with_invalid_prefix() {
        let dir = std::env::temp_dir().join("__resilient_pkg_badconstraint_prefix");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "1.0.0"
[dependencies]
utils = ">=1.0.0"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("invalid constraint prefix must fail");
        assert!(err.contains("invalid constraint"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_errors_on_constraint_with_malformed_version() {
        let dir = std::env::temp_dir().join("__resilient_pkg_badconstraint_ver");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "1.0.0"
[dependencies]
utils = "^1.0"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("malformed constraint version must fail");
        assert!(err.contains("invalid constraint"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_errors_when_lock_version_violates_caret_constraint() {
        let dir = std::env::temp_dir().join("__resilient_pkg_lock_caret_violation");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "1.0.0"
[dependencies]
utils = "^1.2.0"
"#;
        let lockfile = r#"
utils = "2.0.0"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        std::fs::write(dir.join("resilient.lock"), lockfile).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("caret violation must fail");
        assert!(err.contains("does not satisfy constraint"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_errors_when_lock_version_violates_tilde_constraint() {
        let dir = std::env::temp_dir().join("__resilient_pkg_lock_tilde_violation");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "1.0.0"
[dependencies]
utils = "~1.2.0"
"#;
        let lockfile = r#"
utils = "1.3.0"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        std::fs::write(dir.join("resilient.lock"), lockfile).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("tilde violation must fail");
        assert!(err.contains("does not satisfy constraint"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_errors_when_lock_version_violates_exact_constraint() {
        let dir = std::env::temp_dir().join("__resilient_pkg_lock_exact_violation");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "1.0.0"
[dependencies]
utils = "1.2.3"
"#;
        let lockfile = r#"
utils = "1.2.4"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        std::fs::write(dir.join("resilient.lock"), lockfile).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("exact violation must fail");
        assert!(err.contains("does not satisfy constraint"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Valid baseline cases ──────────────────────────────────────────────────

    #[test]
    fn check_accepts_valid_manifest_with_dependencies() {
        let dir = std::env::temp_dir().join("__resilient_pkg_valid_multi");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "complex-app"
version = "2.5.10"
[dependencies]
logger = "^1.0.0"
utils = "~0.5.0"
crypto = "3.2.1"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        assert!(
            result.is_ok(),
            "valid manifest with multiple deps must pass"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_accepts_version_with_large_numbers() {
        let dir = std::env::temp_dir().join("__resilient_pkg_large_numbers");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "999.888.777"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        assert!(result.is_ok(), "large semver numbers must pass");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_accepts_lock_file_matching_all_constraints() {
        let dir = std::env::temp_dir().join("__resilient_pkg_lock_valid");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"
[package]
name = "myapp"
version = "1.0.0"
[dependencies]
alpha = "^1.5.0"
beta = "~2.0.0"
gamma = "3.1.4"
"#;
        let lockfile = r#"
alpha = "1.6.2"
beta = "2.0.5"
gamma = "3.1.4"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        std::fs::write(dir.join("resilient.lock"), lockfile).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src_path.to_str().unwrap());
        assert!(
            result.is_ok(),
            "valid lockfile matching all constraints must pass"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
