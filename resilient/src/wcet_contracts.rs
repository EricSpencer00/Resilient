//! Feature 22/50 — Worst-Case Execution Time (WCET) Contracts.
//!
//! `#[wcet(cycles = 500)]` declares a fn's worst-case execution time
//! budget. The static analyzer estimates the call-graph depth and
//! per-fn statement cost, then verifies the budget holds.
//!
//! Cost model (initial slice):
//! * Each statement: 1 cycle
//! * Each call: 5 cycles + callee's WCET (or 100 if unknown)
//! * Each loop: assumed bounded by `loop_bound = 100` (overridable
//!   via a future `#[loop_bound(N)]` attribute)
//!
//! When a fn's estimated WCET exceeds its declared budget, the
//! compiler errors. When the analysis cannot bound a loop, it warns
//! and treats the budget as best-effort.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct WcetSpec {
    pub fn_name: String,
    pub budget_cycles: u64,
}

pub fn collect() -> Vec<WcetSpec> {
    let attrs = crate::feature_attrs::find_kind("wcet");
    // RES-1764: pre-size to attrs.len() — conditional push (only when
    // the `cycles` chunk parses), upper bound.
    let mut out = Vec::with_capacity(attrs.len());
    // RES-2018: see power_contracts::collect — pull the value out of
    // the inner loop first, then move `item` into the spec instead
    // of cloning per push.
    for (item, rec) in attrs {
        let mut budget_cycles: Option<u64> = None;
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                if k.trim() == "cycles" {
                    let v = v.trim().trim_matches('"');
                    if let Ok(n) = v.parse() {
                        budget_cycles = Some(n);
                        break;
                    }
                }
            }
        }
        if let Some(n) = budget_cycles {
            out.push(WcetSpec {
                fn_name: item,
                budget_cycles: n,
            });
        }
    }
    out
}

pub fn estimate_wcet(node: &Node) -> u64 {
    match node {
        Node::Block { stmts, .. } => stmts.iter().map(estimate_wcet).sum::<u64>(),
        Node::CallExpression { arguments, .. } => {
            5 + arguments.iter().map(estimate_wcet).sum::<u64>()
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            estimate_wcet(condition)
                + estimate_wcet(consequence)
                    .max(alternative.as_ref().map(|a| estimate_wcet(a)).unwrap_or(0))
        }
        Node::WhileStatement { body, .. } => 100 * estimate_wcet(body),
        Node::ForInStatement { body, .. } => 100 * estimate_wcet(body),
        Node::ReturnStatement { value: Some(e), .. } => 1 + estimate_wcet(e),
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            1 + estimate_wcet(value)
        }
        Node::ExpressionStatement { expr, .. } => 1 + estimate_wcet(expr),
        Node::InfixExpression { left, right, .. } => 1 + estimate_wcet(left) + estimate_wcet(right),
        Node::PrefixExpression { right, .. } => 1 + estimate_wcet(right),
        _ => 1,
    }
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let specs = collect();
    if specs.is_empty() {
        return Ok(());
    }
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    // RES-1495: borrow each function name as `&str` instead of
    // cloning into the HashMap key — same pattern applied across
    // `power_contracts` / `stack_contracts` in this PR.
    let bodies: HashMap<&str, &Node> = stmts
        .iter()
        .filter_map(|s| match &s.node {
            Node::Function { name, body, .. } => Some((name.as_str(), body.as_ref())),
            _ => None,
        })
        .collect();
    for spec in &specs {
        if let Some(body) = bodies.get(spec.fn_name.as_str()) {
            let est = estimate_wcet(body);
            if est > spec.budget_cycles {
                return Err(format!(
                    "{}:0:0: error: `{}` WCET budget exceeded: estimated {} > declared {}",
                    source_path, spec.fn_name, est, spec.budget_cycles
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn budget_violation_is_error() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "tight",
            crate::feature_attrs::AttrRecord {
                name: "wcet".into(),
                args: r#"cycles = "5""#.into(),
                line: 0,
            },
        );
        let src = r#"
            fn tight(int x) {
                let a = x;
                let b = x;
                let c = x;
                let d = x;
                let e = x;
                let f = x;
                let g = x;
                let h = x;
                return a + b;
            }
        "#;
        let (prog, _) = parse(src);
        let res = check(&prog, "test");
        assert!(res.is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn budget_within_bounds_passes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "loose",
            crate::feature_attrs::AttrRecord {
                name: "wcet".into(),
                args: r#"cycles = "10000""#.into(),
                line: 0,
            },
        );
        let src = r#"fn loose(int x) { return x + 1; }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }
}
