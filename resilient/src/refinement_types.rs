//! Feature 12/50 — Refinement Types.
//!
//! `#[refinement(name = "PositiveInt", base = "int", where = "self > 0")]`
//! attached to a `type` alias creates a refinement type: a base type
//! constrained by a Z3-checkable predicate. Unlike a runtime contract,
//! the refinement is part of the type — assigning a value to a
//! refined variable triggers a Z3 obligation that the value satisfies
//! the predicate.
//!
//! This first slice records refinement specs in a process-wide
//! registry. The typechecker integration (call site → obligation) is
//! a downstream PR; what ships here is:
//!
//! 1. The attribute parser (via `feature_attrs`).
//! 2. The spec registry — `RefinementSpec { name, base, predicate }`.
//! 3. A `refine(value, refinement_name)` runtime guard helper that
//!    can be called from generated code (used by tests today).
//! 4. A `--list-refinements` audit surface.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct RefinementSpec {
    pub name: String,
    pub base: String,
    pub predicate: String,
}

static REFINEMENTS: RwLock<Vec<RefinementSpec>> = RwLock::new(Vec::new());

pub fn collect_specs() -> Vec<RefinementSpec> {
    let attrs = crate::feature_attrs::find_kind("refinement");
    let mut out = Vec::new();
    for (item, rec) in attrs {
        let mut spec = RefinementSpec {
            name: item,
            base: String::new(),
            predicate: String::new(),
        };
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "base" | "where" => {
                        if k == "base" {
                            spec.base = v.to_string();
                        } else {
                            spec.predicate = v.to_string();
                        }
                    }
                    "name" => {
                        spec.name = v.to_string();
                    }
                    _ => {}
                }
            }
        }
        out.push(spec);
    }
    out
}

pub fn install(specs: Vec<RefinementSpec>) {
    if let Ok(mut g) = REFINEMENTS.write() {
        *g = specs;
    }
}

pub fn lookup(name: &str) -> Option<RefinementSpec> {
    REFINEMENTS
        .read()
        .ok()
        .and_then(|g| g.iter().find(|s| s.name == name).cloned())
}

/// Trivial runtime guard: evaluates a refinement predicate against an
/// integer. The predicate language is intentionally tiny: the literal
/// `self`, an operator (`>`, `<`, `>=`, `<=`, `==`, `!=`), and an
/// integer literal. Anything more complex falls back to "satisfied"
/// and the Z3 path takes over (downstream PR).
pub fn refine_int(value: i64, refinement_name: &str) -> Result<i64, String> {
    let spec = match lookup(refinement_name) {
        Some(s) => s,
        None => return Ok(value), // unknown refinement: pass through
    };
    let p = spec.predicate.trim();
    let mut parts = p.split_whitespace();
    let lhs = parts.next().unwrap_or("self");
    let op = parts.next().unwrap_or("==");
    let rhs: i64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    if lhs != "self" {
        return Ok(value);
    }
    let ok = match op {
        ">" => value > rhs,
        "<" => value < rhs,
        ">=" => value >= rhs,
        "<=" => value <= rhs,
        "==" => value == rhs,
        "!=" => value != rhs,
        _ => true,
    };
    if ok {
        Ok(value)
    } else {
        Err(format!(
            "refinement `{}` violated: {} {} {} is false",
            spec.name, value, op, rhs
        ))
    }
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1302: skip the `install` call when the current program
    // declares no `#[refinement]` attributes. The `install` helper
    // *replaces* the process-global `REFINEMENTS` vector — calling
    // it with an empty list wipes whatever the previous compilation
    // (or, in `cargo test`, a parallel test that called `install`
    // directly under `feature_attrs::lock_for_test()`) set.
    //
    // The race: `refine_int_enforces_predicate` holds
    // `feature_attrs::lock_for_test()` and calls `install([Positive])`
    // directly. That lock guards `ATTR_REGISTRY`, not `REFINEMENTS`.
    // A parallel test that parses + typechecks a program with no
    // refinement attribute lands here, where `collect_specs()`
    // returns `[]`, and the unconditional `install(vec![])` clears
    // `REFINEMENTS` between the test's install and its
    // `refine_int(value, "Positive")` lookup. The lookup then
    // returns `None`, `refine_int` passes the value through, and
    // the `assert!(refine_int(-1, "Positive").is_err())` assertion
    // fails. Skipping the call when the input is empty avoids the
    // wipe; production compilation only mutates the global when the
    // *current* program declares refinements, which is the only
    // case the global is needed for that program anyway.
    let specs = collect_specs();
    if specs.is_empty() {
        return Ok(());
    }
    install(specs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_refinement_spec() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "PositiveInt",
            crate::feature_attrs::AttrRecord {
                name: "refinement".into(),
                args: r#"base = "int", where = "self > 0""#.into(),
                line: 0,
            },
        );
        let specs = collect_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "PositiveInt");
        assert_eq!(specs[0].predicate, "self > 0");
        crate::feature_attrs::reset();
    }

    #[test]
    fn refine_int_enforces_predicate() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        install(vec![RefinementSpec {
            name: "Positive".into(),
            base: "int".into(),
            predicate: "self > 0".into(),
        }]);
        assert!(refine_int(5, "Positive").is_ok());
        assert!(refine_int(0, "Positive").is_err());
        assert!(refine_int(-1, "Positive").is_err());
    }

    #[test]
    fn unknown_refinement_passes_through() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        install(vec![]);
        assert_eq!(refine_int(42, "DoesntExist").ok(), Some(42));
    }
}
