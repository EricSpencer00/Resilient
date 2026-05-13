//! Feature 39/50 — Macros (Compile-Time Substitution).
//!
//! `#[macro(pattern = "...", expansion = "...")]` declares a simple
//! syntactic macro: when the parser sees a call to the macro's name,
//! it substitutes the expansion template (with `$arg` placeholders
//! filled in from the call site).
//!
//! This is a textual macro system (not hygienic), suitable for
//! `assert_eq!`, `format!`, and small DSLs. Hygiene + procedural
//! macros are downstream tickets.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone)]
pub struct MacroDef {
    pub name: String,
    pub pattern: String,
    pub expansion: String,
}

static MACROS: LazyLock<RwLock<HashMap<String, MacroDef>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<MacroDef> {
    let attrs = crate::feature_attrs::find_kind("macro");
    // RES-1764: pre-size to attrs.len() — exactly one push per
    // attribute record.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut pattern = String::new();
        let mut expansion = String::new();
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "pattern" => pattern = v.to_string(),
                    "expansion" => expansion = v.to_string(),
                    _ => {}
                }
            }
        }
        out.push(MacroDef {
            name: item,
            pattern,
            expansion,
        });
    }
    out
}

pub fn install(macros: Vec<MacroDef>) {
    if let Ok(mut g) = MACROS.write() {
        g.clear();
        for m in macros {
            g.insert(m.name.clone(), m);
        }
    }
}

pub fn expand(name: &str, args: &[String]) -> Option<String> {
    let g = MACROS.read().ok()?;
    let def = g.get(name)?;
    let mut out = def.expansion.clone();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("${}", i + 1), a);
    }
    Some(out)
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1308: gate `install` on the non-empty case — see RES-1302
    // for the wipe-on-empty race rationale; same pattern saves a
    // wasted RwLock write per compile in the common case.
    let macros = collect();
    if macros.is_empty() {
        return Ok(());
    }
    install(macros);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_assert_eq() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "assert_eq",
            crate::feature_attrs::AttrRecord {
                name: "macro".into(),
                args: r#"pattern = "$1, $2", expansion = "if $1 != $2 { panic(\"not equal\") }""#
                    .into(),
                line: 0,
            },
        );
        install(collect());
        let exp = expand("assert_eq", &["x".into(), "5".into()]).unwrap();
        assert!(exp.contains("if x != 5"));
        crate::feature_attrs::reset();
    }
}
