//! Feature 37/50 — Custom Derives.
//!
//! `#[derive(Debug, Eq, Hash)]` on a struct or enum auto-generates
//! the listed trait impls. The first slice supports a curated list:
//!
//! * `Debug` — `to_string` returning a struct-like Rust-style debug
//!   representation.
//! * `Eq` — pairwise field equality.
//! * `Hash` — combine field hashes via the SipHash default.
//! * `Default` — constructor with primitive defaults.
//!
//! The actual lowering (synthesizing trait impl AST nodes) is a
//! follow-up; this module records what was requested so runtime
//! generic dispatch and the LSP can advertise the derived methods.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone)]
pub struct DeriveSet {
    pub type_name: String,
    pub traits: Vec<String>,
}

static DERIVES: LazyLock<RwLock<HashMap<String, DeriveSet>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

const SUPPORTED: &[&str] = &["Debug", "Eq", "Hash", "Default", "Clone", "Ord"];

pub fn collect() -> Vec<DeriveSet> {
    let attrs = crate::feature_attrs::find_kind("derive");
    let mut out = Vec::new();
    for (item, rec) in attrs {
        let traits: Vec<String> = rec
            .args
            .split(',')
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        out.push(DeriveSet {
            type_name: item,
            traits,
        });
    }
    out
}

pub fn install(sets: Vec<DeriveSet>) {
    if let Ok(mut g) = DERIVES.write() {
        g.clear();
        for s in sets {
            g.insert(s.type_name.clone(), s);
        }
    }
}

pub fn derives_trait(type_name: &str, trait_name: &str) -> bool {
    DERIVES
        .read()
        .ok()
        .and_then(|g| {
            g.get(type_name)
                .map(|s| s.traits.iter().any(|t| t == trait_name))
        })
        .unwrap_or(false)
}

pub(crate) fn check(_program: &Node, source_path: &str) -> Result<(), String> {
    let sets = collect();
    install(sets.clone());
    for s in &sets {
        for t in &s.traits {
            if !SUPPORTED.contains(&t.as_str()) {
                return Err(format!(
                    "{}:0:0: error: `#[derive({})]` on `{}` — unknown trait. Supported: {:?}",
                    source_path, t, s.type_name, SUPPORTED
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_trait_passes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Reading",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Debug, Eq".into(),
                line: 0,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(res.is_ok());
        assert!(derives_trait("Reading", "Debug"));
        assert!(derives_trait("Reading", "Eq"));
        assert!(!derives_trait("Reading", "Hash"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn unknown_trait_errors() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "X",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "BogusTrait".into(),
                line: 0,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(res.is_err());
        crate::feature_attrs::reset();
    }
}
