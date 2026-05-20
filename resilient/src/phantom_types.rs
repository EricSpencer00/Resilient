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

/// RES-2110: dropped the redundant `type_name: String` field. The
/// `SPECS` HashMap already keys each spec by type name, and no
/// caller (in-tree or external) reads `PhantomSpec::type_name` — the
/// only consumers are `unit_of` and `compatible`, both of which look
/// up by `&str` and read the `unit` field. The field was pure
/// duplication and forced `install` to clone the type name for the
/// map key. The new shape pairs `(type_name, spec)` as a tuple in the
/// `Vec` returned by `collect`, letting `install` move the name
/// straight into the map key with zero allocations.
#[derive(Debug, Clone)]
pub struct PhantomSpec {
    pub unit: String,
}

static SPECS: LazyLock<RwLock<HashMap<String, PhantomSpec>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<(String, PhantomSpec)> {
    let attrs = crate::feature_attrs::find_kind("phantom");
    // RES-1754: pre-size to attrs.len() — conditional push (only when
    // the `units` chunk parsed non-empty), so this is an upper bound.
    let mut out = Vec::with_capacity(attrs.len());
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
            out.push((item, PhantomSpec { unit }));
        }
    }
    out
}

pub fn install(specs: Vec<(String, PhantomSpec)>) {
    if let Ok(mut g) = SPECS.write() {
        g.clear();
        for (name, spec) in specs {
            g.insert(name, spec);
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
    // RES-1572: hold the read guard once and compare units in place.
    // The previous shape called `unit_of` twice — each call acquired
    // the `SPECS` RwLock and cloned the unit `String` from the
    // matching spec, only to compare them and drop both. With the
    // read guard held, both `g.get(...)` lookups borrow directly
    // and the comparison runs on `&String` references. Zero clones,
    // one lock acquire per call.
    let Ok(g) = SPECS.read() else {
        return true;
    };
    match (g.get(lhs), g.get(rhs)) {
        (Some(a), Some(b)) => a.unit == b.unit,
        _ => true, // unknown unit pair: defer to base typechecker
    }
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1308: gate `install` on the non-empty case — see RES-1302
    // for the wipe-on-empty race rationale; same pattern saves a
    // wasted RwLock write per compile in the common case.
    let specs = collect();
    if specs.is_empty() {
        return Ok(());
    }
    install(specs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_phantom(type_name: &str, units: &str) {
        crate::feature_attrs::record(
            type_name,
            crate::feature_attrs::AttrRecord {
                name: "phantom".into(),
                args: format!(r#"units = "{units}""#),
                line: 0,
            },
        );
    }

    #[test]
    fn matching_units_compatible() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_phantom("Meters", "Meters");
        record_phantom("Seconds", "Seconds");
        install(collect());
        assert!(compatible("Meters", "Meters"));
        assert!(!compatible("Meters", "Seconds"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn unregistered_types_defer_to_base_typechecker() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        install(collect());
        assert!(
            compatible("Unknown", "AlsoUnknown"),
            "unregistered type pairs must return true to defer to the base typechecker"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn same_units_across_different_types_are_compatible() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_phantom("Kelvin", "Temperature");
        record_phantom("Celsius", "Temperature");
        record_phantom("Amps", "Current");
        install(collect());
        assert!(
            compatible("Kelvin", "Kelvin"),
            "same type must be compatible"
        );
        assert!(
            compatible("Kelvin", "Celsius"),
            "same units must be compatible across types"
        );
        assert!(
            !compatible("Kelvin", "Amps"),
            "different units must not be compatible"
        );
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
