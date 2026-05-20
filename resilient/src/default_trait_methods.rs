//! Feature 35/50 — Default Method Bodies in Traits.
//!
//! Extends `crate::traits` so a trait declaration's method may carry
//! a body that serves as the default implementation:
//!
//! ```text
//! trait Printable {
//!     fn to_string(self) -> string;
//!     fn print(self) { println(self.to_string()); }   // default body
//! }
//! ```
//!
//! The first slice ships:
//! * Recognition: detect default-bodied trait methods in
//!   `Node::TraitDecl`.
//! * Registry: store `(trait_name, method_name) -> Node` mapping
//!   that can be queried by an `impl` block to fill in missing
//!   methods.
//!
//! The full lowering (synthesising the impl method when omitted)
//! is a follow-up PR; today the analyzer reports which trait methods
//! have defaults to validate downstream tooling.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, RwLock};

/// RES-2194: dropped the `DefaultMethod` struct and the flat
/// `HashMap<(String, String), DefaultMethod>` registry shape. The
/// struct's two String fields (`trait_name`, `method_name`)
/// duplicated exactly what the tuple key already encoded, and the
/// only consumer (`has_default`) just returns `bool`. The flat
/// tuple-key shape ALSO forced `has_default` to allocate two
/// transient `String`s per call because stdlib's `Borrow` impls
/// don't allow `(String, String): Borrow<(&str, &str)>`.
///
/// New shape: nested `HashMap<String, HashSet<String>>` (trait → set
/// of default-bodied method names). Two-step lookup with zero
/// allocations on the hot path. Same fix as RES-2008 / RES-2010 /
/// RES-2012 / RES-2014 (nested-map for the rest of the tuple-keyed
/// registries) and RES-2184 (associated_constants — drop value
/// struct that just duplicated keys).
static DEFAULTS: LazyLock<RwLock<HashMap<String, HashSet<String>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn install(items: Vec<(String, String)>) {
    if let Ok(mut g) = DEFAULTS.write() {
        g.clear();
        // RES-2194: move (trait_name, method_name) pairs straight into
        // the nested map. The previous shape per-item cloned both
        // strings to produce the tuple key, then duplicated them
        // again inside the stored DefaultMethod value.
        for (trait_name, method_name) in items {
            g.entry(trait_name).or_default().insert(method_name);
        }
    }
}

pub fn has_default(trait_name: &str, method: &str) -> bool {
    // RES-2194: two-step lookup — `cache.get(&str)` on each level uses
    // the existing `String: Borrow<str>` impl. Zero allocations per
    // call. The previous flat `(String, String)` key forced
    // `(trait_name.to_string(), method.to_string())` per call.
    DEFAULTS
        .read()
        .ok()
        .map(|g| g.get(trait_name).is_some_and(|m| m.contains(method)))
        .unwrap_or(false)
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1306: gate `install` on the non-empty case. `find_kind`
    // returning empty (no `#[default_impl]` attribute anywhere) is
    // the common case; skipping the `install` write avoids the
    // wasted RwLock acquisition and the wipe-on-empty test race
    // shape documented in RES-1302.
    let _ = program;
    let attrs = crate::feature_attrs::find_kind("default_impl");
    if attrs.is_empty() {
        return Ok(());
    }
    // The Node::TraitDecl variant in lib.rs holds method signatures
    // without bodies in the current ABI. As a first slice we discover
    // default bodies via the `#[derive]`-style attribute companion:
    // any trait method named in `#[default_impl(trait="T", method="m")]`
    // is registered here.
    // RES-1784: pre-size to attrs.len() — exactly one push per
    // attribute record.
    let mut items: Vec<(String, String)> = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut trait_name = String::new();
        let mut method_name = item;
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "trait" => trait_name = v.to_string(),
                    "method" => method_name = v.to_string(),
                    _ => {}
                }
            }
        }
        if !trait_name.is_empty() {
            items.push((trait_name, method_name));
        }
    }
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
    fn registers_default_method() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        install(vec![("Printable".into(), "print".into())]);
        assert!(has_default("Printable", "print"));
        assert!(!has_default("Printable", "to_string"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn has_default_returns_false_for_unregistered_trait() {
        assert!(!has_default("NonExistent", "method"));
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
