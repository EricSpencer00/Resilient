//! Feature 10/50 — Intent Blocks.
//!
//! `#[intent("name", property = "...", enforced_by = "fn1, fn2")]`
//! declares a high-level safety property and the set of functions
//! responsible for collectively maintaining it. The compiler verifies
//! that:
//!
//! 1. Every named `enforced_by` fn exists in the program.
//! 2. Each enforcing fn has at least one `requires` or `ensures`
//!    clause (otherwise the intent has nothing to lean on).
//!
//! This is the "specification separate from implementation" feature
//! from the design doc — high-level intents that map to concrete fns.
//! When an intent's enforcer set is incomplete, the compiler warns:
//! the user has named a property without backing it with verifiable
//! contracts.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct IntentSpec {
    pub item_name: String,
    pub raw_args: String,
    pub property: Option<String>,
    pub enforcers: Vec<String>,
}

pub fn collect() -> Vec<IntentSpec> {
    let attrs = crate::feature_attrs::find_kind("intent");
    // RES-1754: pre-size to attrs.len() — exactly one push per attribute record.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut spec = IntentSpec {
            item_name: item,
            raw_args: rec.args.clone(),
            property: None,
            enforcers: Vec::new(),
        };
        // Permissive parsing: split on `=`, look for known keys.
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "property" => spec.property = Some(v.to_string()),
                    "enforced_by" => {
                        spec.enforcers = v.split_whitespace().map(|s| s.to_string()).collect();
                    }
                    _ => {}
                }
            }
        }
        out.push(spec);
    }
    out
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let intents = collect();
    if intents.is_empty() {
        return Ok(());
    }
    let Node::Program(stmts) = program else {
        return Ok(());
    };

    // RES-1517: collect function names and their contract status in one pass.
    // RES-1994: previously kept a parallel `fn_names: HashSet<&str>` whose
    // key set was identical to `fn_contracts` just for the existence
    // check below. Drop it — `fn_contracts.get()` returning `None`
    // already encodes "doesn't exist", and `Some(false)` encodes "no
    // contracts". One HashMap allocation + N inserts per program
    // instead of two of each.
    let mut fn_contracts: HashMap<&str, bool> = HashMap::with_capacity(stmts.len());
    for s in stmts {
        if let Node::Function {
            name,
            requires,
            ensures,
            ..
        } = &s.node
        {
            fn_contracts.insert(name.as_str(), !requires.is_empty() || !ensures.is_empty());
        }
    }

    for intent in &intents {
        let prop_label = intent.property.as_deref().unwrap_or(&intent.item_name);

        for enforcer in &intent.enforcers {
            match fn_contracts.get(enforcer.as_str()) {
                None => {
                    eprintln!(
                        "warning: intent `{}` (property: \"{}\") names enforcer `{}` \
                         which doesn't exist in the program",
                        intent.item_name, prop_label, enforcer
                    );
                }
                Some(false) => {
                    eprintln!(
                        "warning: intent `{}` (property: \"{}\") enforcer `{}` \
                         has no `requires` or `ensures` clauses — the intent \
                         has no verifiable contract to back it",
                        intent.item_name, prop_label, enforcer
                    );
                }
                Some(true) => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_property_and_enforcers() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "TempControl",
            crate::feature_attrs::AttrRecord {
                name: "intent".into(),
                args: r#"property = "stays bounded" , enforced_by = "f1 f2""#.into(),
                line: 0,
            },
        );
        let intents = collect();
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].item_name, "TempControl");
        assert_eq!(intents[0].property.as_deref(), Some("stays bounded"));
        assert_eq!(
            intents[0].enforcers,
            vec!["f1".to_string(), "f2".to_string()]
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn collect_empty_when_no_attrs() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        assert!(collect().is_empty());
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

    #[test]
    fn check_ok_when_enforcer_has_contract() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "SafeDiv",
            crate::feature_attrs::AttrRecord {
                name: "intent".into(),
                args: r#"property = "no div by zero", enforced_by = "divide""#.into(),
                line: 0,
            },
        );
        let src = "fn divide(int a, int b) -> int requires b != 0 { return a / b; }\n";
        let (prog, _) = crate::parse(src);
        // check always returns Ok (warnings only) but the enforcer has a contract
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_ok_even_when_enforcer_missing_contract() {
        // check() always returns Ok; the gap is reported as a warning only.
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Safe",
            crate::feature_attrs::AttrRecord {
                name: "intent".into(),
                args: r#"property = "safe", enforced_by = "naked""#.into(),
                line: 0,
            },
        );
        let src = "fn naked(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }
}
