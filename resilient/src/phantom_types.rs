//! Feature 17/50 — Phantom Types / Units of Measure.
//!
//! `#[phantom(units = "Meters")]` marks a newtype as a phantom type
//! that carries compile-time units without runtime cost. The compiler
//! rejects arithmetic between values of different phantom units
//! (e.g. `Meters + Seconds`) but allows scaling and same-unit ops.
//!
//! This module records the phantom registry and provides a
//! `compatible(lhs, rhs)` API used by the typechecker arithmetic
//! pass.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone)]
pub struct PhantomSpec {
    pub type_name: String,
    pub unit: String,
}

static SPECS: LazyLock<RwLock<HashMap<String, PhantomSpec>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<PhantomSpec> {
    let attrs = crate::feature_attrs::find_kind("phantom");
    let mut out = Vec::new();
    for (item, rec) in attrs {
        let mut unit = String::new();
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                if k.trim() == "units" {
                    unit = v.trim().trim_matches('"').to_string();
                }
            }
        }
        if !unit.is_empty() {
            out.push(PhantomSpec {
                type_name: item,
                unit,
            });
        }
    }
    out
}

pub fn install(specs: Vec<PhantomSpec>) {
    if let Ok(mut g) = SPECS.write() {
        g.clear();
        for s in specs {
            g.insert(s.type_name.clone(), s);
        }
    }
}

pub fn unit_of(type_name: &str) -> Option<String> {
    SPECS
        .read()
        .ok()
        .and_then(|g| g.get(type_name).map(|s| s.unit.clone()))
}

pub fn compatible(lhs: &str, rhs: &str) -> bool {
    match (unit_of(lhs), unit_of(rhs)) {
        (Some(a), Some(b)) => a == b,
        _ => true, // unknown unit pair: defer to base typechecker
    }
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    install(collect());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_units_compatible() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Meters",
            crate::feature_attrs::AttrRecord {
                name: "phantom".into(),
                args: r#"units = "Meters""#.into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "Seconds",
            crate::feature_attrs::AttrRecord {
                name: "phantom".into(),
                args: r#"units = "Seconds""#.into(),
                line: 0,
            },
        );
        install(collect());
        assert!(compatible("Meters", "Meters"));
        assert!(!compatible("Meters", "Seconds"));
        crate::feature_attrs::reset();
    }
}
