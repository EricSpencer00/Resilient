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

#[derive(Debug, Clone)]
pub struct AssocConstant {
    pub type_name: String,
    pub trait_name: String,
    pub const_name: String,
    pub value: String,
}

static ASSOC: LazyLock<RwLock<HashMap<(String, String), AssocConstant>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<AssocConstant> {
    let attrs = crate::feature_attrs::find_kind("assoc_const");
    let mut out = Vec::new();
    for (item, rec) in attrs {
        let mut tr = String::new();
        let mut name = String::new();
        let mut val = String::new();
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "trait" => tr = v.to_string(),
                    "name" => name = v.to_string(),
                    "value" => val = v.to_string(),
                    _ => {}
                }
            }
        }
        if !name.is_empty() {
            out.push(AssocConstant {
                type_name: item,
                trait_name: tr,
                const_name: name,
                value: val,
            });
        }
    }
    out
}

pub fn install(items: Vec<AssocConstant>) {
    if let Ok(mut g) = ASSOC.write() {
        g.clear();
        for a in items {
            g.insert((a.type_name.clone(), a.const_name.clone()), a);
        }
    }
}

pub fn lookup(type_name: &str, const_name: &str) -> Option<String> {
    ASSOC.read().ok().and_then(|g| {
        g.get(&(type_name.to_string(), const_name.to_string()))
            .map(|a| a.value.clone())
    })
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    install(collect());
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
}
