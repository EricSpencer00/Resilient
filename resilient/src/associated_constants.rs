//! Feature 36/50 — Associated Constants in Traits.
//!
//! Trait-associated constants:
//!
//! ```text
//! trait Bounded { const MIN: int; const MAX: int; }
//! impl Bounded for Temperature {
//!     const MIN: int = -40;
//!     const MAX: int = 125;
//! }
//! ```
//!
//! Recorded as attributes today: `#[assoc_const(trait="Bounded", name="MIN", value="-40")]`
//! on a struct registers an associated constant. The runtime / typechecker
//! can resolve `Temperature::MIN` from the registry.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// RES-2184: dropped the `AssocConstant` struct entirely. Its three
/// name fields (`type_name`, `trait_name`, `const_name`) had zero
/// readers after install — `lookup` only returns `a.value.clone()`,
/// and external grep returned no other consumers. The registry now
/// stores the value directly. The collect/install pipeline carries
/// `(type, const, value)` triples; `trait` is still parsed (for
/// backward compat with existing attribute syntax) but discarded.
/// Same dead-field-cleanup sentiment as the RES-2106 / … / RES-2182
/// series, applied more aggressively here since *three* fields per
/// entry were unread.
///
/// RES-2014: nested map — outer key `type_name`, inner key `const_name`.
/// The flat `HashMap<(String, String), V>` shape forced `lookup` to
/// allocate two transient `String`s per call (stdlib's `Borrow`
/// impls don't allow `(String, String): Borrow<(&str, &str)>`).
/// Both nested-map `.get` calls accept `&str` via the existing
/// `String: Borrow<str>` impl. Same fix as RES-2008 / RES-2010 /
/// RES-2012 — completes the (String, String) HashMap key conversion
/// across all four registries in the codebase.
static ASSOC: LazyLock<RwLock<HashMap<String, HashMap<String, String>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Returns `(type_name, const_name, value)` triples.
pub fn collect() -> Vec<(String, String, String)> {
    let attrs = crate::feature_attrs::find_kind("assoc_const");
    // RES-1782: pre-size to attrs.len() — at most one push per
    // attribute record (skipped when `name` doesn't parse).
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut name = String::new();
        let mut val = String::new();
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "name" => name = v.to_string(),
                    "value" => val = v.to_string(),
                    // "trait" key is accepted in the attribute syntax
                    // for forward-compat but has no reader anywhere;
                    // discard without allocating.
                    _ => {}
                }
            }
        }
        if !name.is_empty() {
            out.push((item, name, val));
        }
    }
    out
}

pub fn install(items: Vec<(String, String, String)>) {
    if let Ok(mut g) = ASSOC.write() {
        g.clear();
        // RES-2184: move `type_name` into the outer `entry` slot (no
        // clone) and `(const_name, value)` straight into the inner
        // map. The previous shape per-item cloned `a.type_name` and
        // `a.const_name` to produce keys whose values then duplicated
        // those same strings inside the stored `AssocConstant`.
        for (type_name, const_name, value) in items {
            g.entry(type_name).or_default().insert(const_name, value);
        }
    }
}

pub fn lookup(type_name: &str, const_name: &str) -> Option<String> {
    // RES-2014: nested-map lookup — `.get(&str)` on each level uses
    // the existing `String: Borrow<str>` impl. Zero per-call
    // allocations (the previous flat `(String, String)` key forced
    // two transient `String::to_string()` allocs per call).
    ASSOC
        .read()
        .ok()
        .and_then(|g| g.get(type_name).and_then(|m| m.get(const_name)).cloned())
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1308: gate `install` on the non-empty case. The previous
    // wiring wrote to `ASSOC` on every typecheck, burning a
    // RwLock write acquisition + replace on every program that
    // declares no `#[assoc_const]` attribute. It also created the
    // wipe-on-empty test race documented in RES-1302 against any
    // test that calls `install` directly under
    // `feature_attrs::lock_for_test()`.
    let items = collect();
    if items.is_empty() {
        return Ok(());
    }
    install(items);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_returns_value() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Temperature",
            crate::feature_attrs::AttrRecord {
                name: "assoc_const".into(),
                args: r#"trait = "Bounded", name = "MIN", value = "-40""#.into(),
                line: 0,
            },
        );
        install(collect());
        assert_eq!(lookup("Temperature", "MIN"), Some("-40".to_string()));
        crate::feature_attrs::reset();
    }

    #[test]
    fn lookup_missing_returns_none() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        install(collect());
        assert_eq!(
            lookup("NotRegistered", "ANYTHING"),
            None,
            "unregistered type+const must return None"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn multiple_constants_on_same_type() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Volt",
            crate::feature_attrs::AttrRecord {
                name: "assoc_const".into(),
                args: r#"trait = "Units", name = "UNIT", value = "V""#.into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "Volt",
            crate::feature_attrs::AttrRecord {
                name: "assoc_const".into(),
                args: r#"trait = "Bounded", name = "MAX", value = "48""#.into(),
                line: 0,
            },
        );
        install(collect());
        assert_eq!(lookup("Volt", "UNIT"), Some("V".to_string()));
        assert_eq!(lookup("Volt", "MAX"), Some("48".to_string()));
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }
}
