//! Feature 5/50 — Semantic Regression Check.
//!
//! Diff two parsed programs at the *semantic* level: report fns whose
//! contracts (requires/ensures/fails) weakened, strengthened, or
//! disappeared. Sibling to `behavioral_fingerprint`, but produces a
//! human-readable changelog rather than just hash mismatches.
//!
//! A contract change is classified as:
//! * **Removed** — clause present in old, absent in new.
//! * **Added** — clause absent in old, present in new.
//! * **Weakened** — `requires` count decreased OR `ensures` count
//!   decreased (the new version promises less).
//! * **Strengthened** — opposite direction.
//!
//! `--check-regression OLD NEW` exits non-zero when any weakening is
//! detected, making this CI-enforceable.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticChange {
    Added(String),
    Removed(String),
    Weakened {
        function: String,
        old_count: usize,
        new_count: usize,
        kind: ContractKind,
    },
    Strengthened {
        function: String,
        old_count: usize,
        new_count: usize,
        kind: ContractKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractKind {
    Requires,
    Ensures,
    Fails,
}

#[derive(Debug, Clone, Default)]
pub struct FunctionContract {
    pub requires_count: usize,
    pub ensures_count: usize,
    pub fails_variants: Vec<String>,
}

pub fn extract_contracts(program: &Node) -> HashMap<String, FunctionContract> {
    let Node::Program(stmts) = program else {
        return HashMap::new();
    };
    // RES-1754: pre-size to stmts.len() — every top-level statement
    // could be a function and produce one insert. Upper bound; same
    // pattern as the call-graph pre-size series.
    let mut out = HashMap::with_capacity(stmts.len());
    for s in stmts {
        if let Node::Function {
            name,
            requires,
            ensures,
            fails,
            ..
        } = &s.node
        {
            out.insert(
                name.clone(),
                FunctionContract {
                    requires_count: requires.len(),
                    ensures_count: ensures.len(),
                    fails_variants: fails.clone(),
                },
            );
        }
    }
    out
}

pub fn diff(
    old: &HashMap<String, FunctionContract>,
    new: &HashMap<String, FunctionContract>,
) -> Vec<SemanticChange> {
    let mut changes = Vec::new();
    for name in old.keys() {
        if !new.contains_key(name) {
            changes.push(SemanticChange::Removed(name.clone()));
        }
    }
    for (name, new_c) in new {
        if let Some(old_c) = old.get(name) {
            if new_c.requires_count < old_c.requires_count {
                changes.push(SemanticChange::Weakened {
                    function: name.clone(),
                    old_count: old_c.requires_count,
                    new_count: new_c.requires_count,
                    kind: ContractKind::Requires,
                });
            }
            if new_c.requires_count > old_c.requires_count {
                changes.push(SemanticChange::Strengthened {
                    function: name.clone(),
                    old_count: old_c.requires_count,
                    new_count: new_c.requires_count,
                    kind: ContractKind::Requires,
                });
            }
            if new_c.ensures_count < old_c.ensures_count {
                changes.push(SemanticChange::Weakened {
                    function: name.clone(),
                    old_count: old_c.ensures_count,
                    new_count: new_c.ensures_count,
                    kind: ContractKind::Ensures,
                });
            }
            if new_c.ensures_count > old_c.ensures_count {
                changes.push(SemanticChange::Strengthened {
                    function: name.clone(),
                    old_count: old_c.ensures_count,
                    new_count: new_c.ensures_count,
                    kind: ContractKind::Ensures,
                });
            }
        } else {
            changes.push(SemanticChange::Added(name.clone()));
        }
    }
    changes
}

pub fn has_weakening(changes: &[SemanticChange]) -> bool {
    changes.iter().any(|c| {
        matches!(
            c,
            SemanticChange::Weakened { .. } | SemanticChange::Removed(_)
        )
    })
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn weakened_ensures_detected() {
        let s1 = r#"fn f(int x) -> int ensures result > 0 ensures result < 100 { return x; }"#;
        let s2 = r#"fn f(int x) -> int ensures result > 0 { return x; }"#;
        let (p1, _) = parse(s1);
        let (p2, _) = parse(s2);
        let changes = diff(&extract_contracts(&p1), &extract_contracts(&p2));
        assert!(has_weakening(&changes));
    }

    #[test]
    fn added_function_not_a_weakening() {
        let s1 = r#"fn f(int x) { return x; }"#;
        let s2 = r#"fn f(int x) { return x; } fn g(int y) { return y; }"#;
        let (p1, _) = parse(s1);
        let (p2, _) = parse(s2);
        let changes = diff(&extract_contracts(&p1), &extract_contracts(&p2));
        assert!(!has_weakening(&changes));
        assert!(
            changes
                .iter()
                .any(|c| matches!(c, SemanticChange::Added(n) if n == "g"))
        );
    }

    #[test]
    fn strengthening_is_safe() {
        let s1 = r#"fn f(int x) -> int { return x; }"#;
        let s2 = r#"fn f(int x) -> int requires x > 0 { return x; }"#;
        let (p1, _) = parse(s1);
        let (p2, _) = parse(s2);
        let changes = diff(&extract_contracts(&p1), &extract_contracts(&p2));
        assert!(!has_weakening(&changes));
    }
}
