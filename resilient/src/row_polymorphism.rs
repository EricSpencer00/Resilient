//! Feature 15/50 — Row Polymorphism.
//!
//! `#[row_poly(requires = "name:string level:int")]` on a function
//! declares that any caller may pass *any* struct provided it
//! contains at least the listed fields. This is structural
//! subtyping at the function-parameter granularity, no inheritance
//! or interface declaration required.
//!
//! This first slice records the row constraint per function and
//! offers a `validate(fn_name, struct_fields)` query that the
//! typechecker / runtime can consult.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone)]
pub struct RowSpec {
    pub fn_name: String,
    /// Required (field_name, type_name) pairs.
    pub required: Vec<(String, String)>,
}

static SPECS: LazyLock<RwLock<HashMap<String, RowSpec>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<RowSpec> {
    let attrs = crate::feature_attrs::find_kind("row_poly");
    let mut out = Vec::new();
    for (item, rec) in attrs {
        let mut spec = RowSpec {
            fn_name: item,
            required: Vec::new(),
        };
        if let Some(rest) = rec.args.split_once('=').map(|(_, r)| r) {
            let v = rest.trim().trim_matches('"');
            for chunk in v.split_whitespace() {
                if let Some((name, ty)) = chunk.split_once(':') {
                    spec.required.push((name.to_string(), ty.to_string()));
                }
            }
        }
        out.push(spec);
    }
    out
}

pub fn install(specs: Vec<RowSpec>) {
    if let Ok(mut g) = SPECS.write() {
        g.clear();
        for s in specs {
            g.insert(s.fn_name.clone(), s);
        }
    }
}

pub fn validate(fn_name: &str, fields: &[(String, String)]) -> Result<(), String> {
    let specs = SPECS.read().ok().map(|g| g.clone()).unwrap_or_default();
    let spec = match specs.get(fn_name) {
        Some(s) => s,
        None => return Ok(()),
    };
    for (req_name, req_ty) in &spec.required {
        let found = fields.iter().any(|(n, t)| n == req_name && t == req_ty);
        if !found {
            return Err(format!(
                "row-poly violation: fn `{fn_name}` requires field `{req_name}: {req_ty}`"
            ));
        }
    }
    Ok(())
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

    #[test]
    fn validates_minimum_field_set() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "log",
            crate::feature_attrs::AttrRecord {
                name: "row_poly".into(),
                args: r#"requires = "name:string level:int""#.into(),
                line: 0,
            },
        );
        install(collect());
        let ok_fields = vec![
            ("name".to_string(), "string".to_string()),
            ("level".to_string(), "int".to_string()),
            ("ts".to_string(), "int".to_string()),
        ];
        assert!(validate("log", &ok_fields).is_ok());
        let bad = vec![("name".to_string(), "string".to_string())];
        assert!(validate("log", &bad).is_err());
        crate::feature_attrs::reset();
    }
}
