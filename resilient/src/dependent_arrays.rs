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
    let mut out = Vec::new();
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
    install(specs.clone());
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::Function {
            name, type_params, ..
        } = &s.node
        {
            if let Some(spec) = specs.iter().find(|sp| sp.item_name == *name) {
                if !type_params.contains(&spec.length_var) {
                    eprintln!(
                        "warning: `{}` has `#[dependent(length = \"{}\")]` but `{}` is not declared as a generic parameter",
                        name, spec.length_var, spec.length_var
                    );
                }
            }
        }
    }
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
}
