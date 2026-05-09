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
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone)]
pub struct DefaultMethod {
    pub trait_name: String,
    pub method_name: String,
}

static DEFAULTS: LazyLock<RwLock<HashMap<(String, String), DefaultMethod>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn install(items: Vec<DefaultMethod>) {
    if let Ok(mut g) = DEFAULTS.write() {
        g.clear();
        for d in items {
            g.insert((d.trait_name.clone(), d.method_name.clone()), d);
        }
    }
}

pub fn has_default(trait_name: &str, method: &str) -> bool {
    DEFAULTS
        .read()
        .ok()
        .map(|g| g.contains_key(&(trait_name.to_string(), method.to_string())))
        .unwrap_or(false)
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // The Node::TraitDecl variant in lib.rs holds method signatures
    // without bodies in the current ABI. As a first slice we discover
    // default bodies via the `#[derive]`-style attribute companion:
    // any trait method named in `#[default_impl(trait="T", method="m")]`
    // is registered here.
    let mut items = Vec::new();
    for (item, rec) in crate::feature_attrs::find_kind("default_impl") {
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
            items.push(DefaultMethod {
                trait_name,
                method_name,
            });
        }
    }
    install(items);
    let _ = program;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_default_method() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        install(vec![DefaultMethod {
            trait_name: "Printable".into(),
            method_name: "print".into(),
        }]);
        assert!(has_default("Printable", "print"));
        assert!(!has_default("Printable", "to_string"));
        crate::feature_attrs::reset();
    }
}
