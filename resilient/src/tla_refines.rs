//! RES-3930 Phase B1 — external TLA+ refinement mappings.
//!
//! The parser owns the `@refines(spec = "...", action = "...")` syntax and
//! records validated mappings in the shared attribute registry. This module
//! checks that the records still point at named functions and that each
//! function has at most one mapping. TLA+ file loading, action discovery, and
//! proof checking are intentionally left to the later Phase B increments.

use crate::Node;
use std::collections::{HashMap, HashSet};

const ATTRIBUTE: &str = "refines";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RefinesMapping {
    function: String,
    spec: String,
    action: String,
    line: usize,
}

fn parse_record(
    function: String,
    record: &crate::feature_attrs::AttrRecord,
) -> Result<RefinesMapping, String> {
    let mut values = HashMap::new();
    for entry in record.args.lines() {
        let Some((key, value)) = entry.split_once('=') else {
            return Err(format!("malformed @refines registry entry `{}`", entry));
        };
        let key = key.trim();
        let value = value.trim();
        if !matches!(key, "spec" | "action") {
            return Err(format!("unknown @refines registry key `{}`", key));
        }
        if value.is_empty() {
            return Err(format!("@refines `{}` value cannot be empty", key));
        }
        if values.insert(key, value.to_string()).is_some() {
            return Err(format!("duplicate @refines registry key `{}`", key));
        }
    }

    let spec = values
        .remove("spec")
        .ok_or_else(|| "@refines registry entry is missing `spec`".to_string())?;
    let action = values
        .remove("action")
        .ok_or_else(|| "@refines registry entry is missing `action`".to_string())?;
    Ok(RefinesMapping {
        function,
        spec,
        action,
        line: record.line,
    })
}

fn diagnostic(source_path: &str, line: usize, message: &str) -> String {
    if line == 0 {
        format!("{}: {}", source_path, message)
    } else {
        format!("{}:{}:1: {}", source_path, line, message)
    }
}

/// Validate the parser-preserved mappings against the current top-level AST.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let records = crate::feature_attrs::find_kind(ATTRIBUTE);
    if records.is_empty() {
        return Ok(());
    }

    let functions: HashSet<&str> = match program {
        Node::Program(statements) => statements
            .iter()
            .filter_map(|statement| match &statement.node {
                Node::Function { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect(),
        _ => HashSet::new(),
    };

    let mut seen = HashSet::with_capacity(records.len());
    let mut mappings = Vec::with_capacity(records.len());
    for (function, record) in records {
        let mapping = parse_record(function, &record)
            .map_err(|message| diagnostic(source_path, record.line, &message))?;
        if !functions.contains(mapping.function.as_str()) {
            return Err(diagnostic(
                source_path,
                mapping.line,
                &format!(
                    "@refines target `{}` is not a top-level function",
                    mapping.function
                ),
            ));
        }
        if !seen.insert(mapping.function.clone()) {
            return Err(diagnostic(
                source_path,
                mapping.line,
                &format!(
                    "function `{}` has more than one @refines mapping",
                    mapping.function
                ),
            ));
        }
        mappings.push(mapping);
    }

    // Keep the structured values live in this first increment. The next
    // checker will consume this same validated representation when it starts
    // resolving specs and actions.
    let _ = mappings;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(args: &str) -> crate::feature_attrs::AttrRecord {
        crate::feature_attrs::AttrRecord {
            name: ATTRIBUTE.to_string(),
            args: args.to_string(),
            line: 7,
        }
    }

    #[test]
    fn parses_normalized_mapping() {
        let mapping = parse_record(
            "increment".to_string(),
            &record("spec=counter.tla\naction=Inc"),
        )
        .expect("normalized parser record should validate");
        assert_eq!(
            mapping,
            RefinesMapping {
                function: "increment".to_string(),
                spec: "counter.tla".to_string(),
                action: "Inc".to_string(),
                line: 7,
            }
        );
    }

    #[test]
    fn rejects_missing_required_mapping_key() {
        let error = parse_record("increment".to_string(), &record("spec=counter.tla"))
            .expect_err("missing action must fail");
        assert!(error.contains("missing `action`"), "{error}");
    }

    #[test]
    fn rejects_duplicate_mapping_key() {
        let error = parse_record(
            "increment".to_string(),
            &record("spec=counter.tla\nspec=other.tla\naction=Inc"),
        )
        .expect_err("duplicate keys must fail");
        assert!(error.contains("duplicate"), "{error}");
    }
}
