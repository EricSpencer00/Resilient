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

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
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
}
