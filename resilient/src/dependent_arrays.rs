//! Feature 14/50 — Dependent Array Types.
//!
//! `#[dependent(length = "N")]` annotated on a function parameter
//! declares that the array's length is the compile-time value `N`,
//! enabling compile-time bound elimination and length-preserving
//! type signatures (e.g. `concat<M, N>(Array<T, M>, Array<T, N>)
//! -> Array<T, M+N>`).
//!
//! This first slice records the dependent specs and provides a
//! `lookup(fn_name, param)` API the typechecker / Z3 backend will
//! consume in a follow-up. Today the analyzer:
//!
//! * Parses the spec from `#[dependent(...)]` attributes.
//! * Verifies that the named length parameter (`N`) appears in the
//!   function's `type_params` list. Otherwise it warns.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone)]
pub struct DependentSpec {
    pub item_name: String,
    pub length_var: String,
}

static SPECS: LazyLock<RwLock<HashMap<String, DependentSpec>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<DependentSpec> {
    let attrs = crate::feature_attrs::find_kind("dependent");
    // RES-1764: pre-size to attrs.len() — conditional push (only when
    // the `length` chunk parsed non-empty), so this is an upper bound.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut length = String::new();
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                if k.trim() == "length" {
                    length = v.trim().trim_matches('"').to_string();
                }
            }
        }
        if !length.is_empty() {
            out.push(DependentSpec {
                item_name: item,
                length_var: length,
            });
        }
    }
    out
}

pub fn install(specs: Vec<DependentSpec>) {
    if let Ok(mut g) = SPECS.write() {
        g.clear();
        for s in specs {
            g.insert(s.item_name.clone(), s);
        }
    }
}

pub fn lookup(item: &str) -> Option<DependentSpec> {
    SPECS.read().ok().and_then(|g| g.get(item).cloned())
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let specs = collect();
    // RES-1495: validate against the local `specs` Vec BEFORE
    // installing into the global `SPECS` registry, then move `specs`
    // into `install` at the end. The previous shape did
    // `install(specs.clone())` up front — paying a `Vec` clone for
    // a registry the validation loop never reads through. The
    // semantics of "install always runs to clear stale state" are
    // preserved: on the early-Program-bail and at the end of the fn,
    // `install` fires with the (possibly empty) collected specs.
    // Same pattern as RES-1481 / RES-1485 / RES-1487 / RES-1489 /
    // RES-1491.
    let Node::Program(stmts) = program else {
        install(specs);
        return Ok(());
    };
    if !specs.is_empty() {
        for s in stmts {
            if let Node::Function {
                name, type_params, ..
            } = &s.node
                && let Some(spec) = specs.iter().find(|sp| sp.item_name == *name)
                && !type_params.contains(&spec.length_var)
            {
                eprintln!(
                    "warning: `{}` has `#[dependent(length = \"{}\")]` but `{}` is not declared as a generic parameter",
                    name, spec.length_var, spec.length_var
                );
            }
        }
    }
    install(specs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_length_var() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "concat",
            crate::feature_attrs::AttrRecord {
                name: "dependent".into(),
                args: r#"length = "M+N""#.into(),
                line: 0,
            },
        );
        let s = collect();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].length_var, "M+N");
        crate::feature_attrs::reset();
    }

    #[test]
    fn collect_returns_empty_without_attribute() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let s = collect();
        assert!(
            s.is_empty(),
            "collect() must return empty vec when no #[dependent] attributes exist"
        );
    }

    #[test]
    fn install_and_lookup_round_trip() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "zip",
            crate::feature_attrs::AttrRecord {
                name: "dependent".into(),
                args: r#"length = "N""#.into(),
                line: 0,
            },
        );
        install(collect());
        let spec = lookup("zip").expect("zip must be found after install");
        assert_eq!(spec.item_name, "zip");
        assert_eq!(spec.length_var, "N");
        assert!(
            lookup("nonexistent").is_none(),
            "lookup must return None for unknown function"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "check() must return Ok when no #[dependent] attributes exist"
        );
    }

    #[test]
    fn missing_length_key_is_ignored() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "bad",
            crate::feature_attrs::AttrRecord {
                name: "dependent".into(),
                args: r#"size = "N""#.into(),
                line: 0,
            },
        );
        let s = collect();
        assert!(
            s.is_empty(),
            "attribute without `length` key must be ignored; got {s:?}"
        );
        crate::feature_attrs::reset();
    }
}
