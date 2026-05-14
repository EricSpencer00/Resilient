//! Feature 37/50 — Custom Derives.
//!
//! `#[derive(Debug, Eq, Hash)]` on a struct or enum auto-generates
//! the listed trait impls. The first slice supports a curated list:
//!
//! * `Debug` — `to_string` returning a struct-like Rust-style debug
//!   representation.
//! * `Eq` / `PartialEq` — pairwise field equality; enables `==`/`!=`.
//! * `Ord` / `PartialOrd` — lexicographic field ordering; enables `<`/`>`.
//! * `Hash` — combine field hashes via the SipHash default.
//! * `Default` — constructor with primitive defaults.
//! * `Clone` / `Copy` — value duplication semantics.
//! * `Display` — human-readable `to_string` with field names.
//! * `Iterator` / `From` / `Into` — standard conversion traits.
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

const SUPPORTED: &[&str] = &[
    "Debug",
    "Eq",
    "Hash",
    "Default",
    "Clone",
    "Ord",
    "PartialEq",
    "PartialOrd",
    "Display",
    "Iterator",
    "From",
    "Into",
    "Copy",
];

pub fn collect() -> Vec<DeriveSet> {
    let attrs = crate::feature_attrs::find_kind("derive");
    // RES-1782: pre-size to attrs.len() — exactly one push per
    // attribute record.
    let mut out = Vec::with_capacity(attrs.len());
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
    // RES-1402: gate `install` on the non-empty case. The historical
    // wiring called `install(sets.clone())` before the trait-validation
    // loop, burning a `DERIVES.write()` lock + `g.clear()` per compile
    // regardless of whether any `#[derive]` attribute was present, AND
    // creating the wipe-on-empty test race shape documented in
    // RES-1302: a parallel test that called `install(...)` directly
    // under `feature_attrs::lock_for_test()` would have its registry
    // wiped by a concurrent typecheck whose `collect()` returned empty.
    // Same pattern as RES-1306 / RES-1308 already applied to
    // `async_await`, `default_trait_methods`, `mmio_regmap`,
    // `distributed_invariants`, `ghost_types`, and friends.
    // RES-1481: validate the trait set before `install` so we can
    // move `sets` into `install` instead of cloning. The previous
    // shape did `install(sets.clone())` ahead of the validation
    // loop, burning a full Vec<DeriveSpec> clone per compile that
    // had `#[derive]` attributes — the clone was thrown away once
    // the for-loop finished iterating `&sets`. As a side benefit,
    // an invalid-trait error now leaves the registry untouched
    // rather than polluting it with an entry that then fails.
    let sets = collect();
    if sets.is_empty() {
        return Ok(());
    }
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
    install(sets);
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

    #[test]
    fn partial_eq_and_partial_ord_accepted() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Point",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "PartialEq, PartialOrd, Display".into(),
                line: 0,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(res.is_ok(), "expected ok, got {:?}", res);
        assert!(derives_trait("Point", "PartialEq"));
        assert!(derives_trait("Point", "PartialOrd"));
        assert!(derives_trait("Point", "Display"));
        assert!(!derives_trait("Point", "Iterator"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn copy_and_iterator_accepted() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "MyIter",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Copy, Iterator, From, Into".into(),
                line: 0,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(res.is_ok(), "expected ok, got {:?}", res);
        assert!(derives_trait("MyIter", "Copy"));
        assert!(derives_trait("MyIter", "Iterator"));
        crate::feature_attrs::reset();
    }
}
