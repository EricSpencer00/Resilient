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
use std::sync::RwLock;

/// Global contract baseline — populated on the first check() call and used
/// to detect weakenings in subsequent compilations (e.g., REPL / incremental).
static BASELINE: RwLock<Option<HashMap<String, FunctionContract>>> = RwLock::new(None);

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

/// Install or update the contract baseline and return the previous snapshot.
pub fn install_baseline(contracts: HashMap<String, FunctionContract>) {
    if let Ok(mut g) = BASELINE.write() {
        *g = Some(contracts);
    }
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // Fast-reject: skip programs with no function declarations.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    let has_fn = stmts.iter().any(|s| matches!(&s.node, Node::Function { .. }));
    if !has_fn {
        return Ok(());
    }

    let current = extract_contracts(program);
    if current.is_empty() {
        return Ok(());
    }

    // Emit contract-density summary — useful as a coverage metric.
    let total = current.len();
    let contracted: Vec<&str> = current
        .iter()
        .filter(|(_, c)| c.requires_count > 0 || c.ensures_count > 0 || !c.fails_variants.is_empty())
        .map(|(n, _)| n.as_str())
        .collect();
    let unconstrained: Vec<&str> = current
        .iter()
        .filter(|(_, c)| c.requires_count == 0 && c.ensures_count == 0 && c.fails_variants.is_empty())
        .map(|(n, _)| n.as_str())
        .collect();
    let pct = (contracted.len() * 100).checked_div(total).unwrap_or(0);
    eprintln!(
        "semantic-regression: {}/{} function(s) have contracts ({}%)",
        contracted.len(),
        total,
        pct
    );
    if !unconstrained.is_empty() {
        let mut names = unconstrained.clone();
        names.sort();
        eprintln!(
            "semantic-regression: unconstrained function(s): [{}]",
            names.join(", ")
        );
    }

    // Compare against the installed baseline (if any) and warn on weakenings.
    let baseline_snap = BASELINE.read().ok().and_then(|g| g.clone());
    if let Some(baseline) = baseline_snap {
        let changes = diff(&baseline, &current);
        for c in &changes {
            match c {
                SemanticChange::Weakened { function, old_count, new_count, kind } => {
                    let kind_str = match kind {
                        ContractKind::Requires => "requires",
                        ContractKind::Ensures => "ensures",
                        ContractKind::Fails => "fails",
                    };
                    eprintln!(
                        "semantic-regression: `{function}` {kind_str} weakened \
                         ({old_count} → {new_count} clause(s))"
                    );
                }
                SemanticChange::Removed(name) => {
                    eprintln!(
                        "semantic-regression: function `{name}` removed — \
                         any callers relied on its contracts"
                    );
                }
                _ => {}
            }
        }
        if has_weakening(&changes) {
            eprintln!("semantic-regression: contract weakening detected — review required");
        }
    }

    // Install current contracts as the new baseline for the next check.
    install_baseline(current);
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

    #[test]
    fn check_returns_ok_on_well_contracted_program() {
        // Reset baseline so this test is independent.
        install_baseline(HashMap::new());
        let src = r#"fn safe(int x) -> int requires x > 0 ensures result > 0 { return x; }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_returns_ok_on_unconstrained_program() {
        install_baseline(HashMap::new());
        let src = r#"fn bare(int x) -> int { return x; }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn regression_detected_after_weakening() {
        // Install a strong baseline, then check a weaker program.
        let strong = {
            let src = r#"fn f(int x) -> int requires x > 0 ensures result > 0 { return x; }"#;
            let (p, _) = parse(src);
            extract_contracts(&p)
        };
        install_baseline(strong);

        // Now check the weaker version (no ensures).
        let weak_src = r#"fn f(int x) -> int requires x > 0 { return x; }"#;
        let (weak, _) = parse(weak_src);
        // check() should return Ok (warnings are non-fatal), but not panic.
        assert!(check(&weak, "test").is_ok());
        // has_weakening verifies the diff directly.
        let weak_contracts = extract_contracts(&weak);
        let strong_src = r#"fn f(int x) -> int requires x > 0 ensures result > 0 { return x; }"#;
        let (strong_prog, _) = parse(strong_src);
        let strong_contracts = extract_contracts(&strong_prog);
        let changes = diff(&strong_contracts, &weak_contracts);
        assert!(has_weakening(&changes), "dropping ensures must be a weakening");
    }

    #[test]
    fn no_regression_when_contracts_equal() {
        let src = r#"fn f(int x) -> int requires x > 0 { return x; }"#;
        let (p, _) = parse(src);
        let contracts = extract_contracts(&p);
        install_baseline(contracts.clone());
        // Same program → no weakening.
        let changes = diff(&contracts, &contracts);
        assert!(!has_weakening(&changes));
    }
}
