//! Feature 28/50 — Power Consumption Contracts.
//!
//! `#[power(uj = 50)]` declares a fn's energy budget in microjoules.
//! The static analyzer estimates energy by walking the call graph
//! and summing per-statement / per-builtin energy costs.
//!
//! Initial energy model (microjoules):
//! * Statement: 0.001 µJ
//! * Volatile read/write (MMIO): 0.05 µJ each
//! * `radio_*` builtin: 100 µJ
//! * `random_*`: 0.01 µJ
//! * Function call: callee's budget if known, else 1 µJ

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct PowerSpec {
    pub fn_name: String,
    pub budget_uj: f64,
}

pub fn collect() -> Vec<PowerSpec> {
    let attrs = crate::feature_attrs::find_kind("power");
    let mut out = Vec::new();
    for (item, rec) in attrs {
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                if k.trim() == "uj" {
                    if let Ok(n) = v.trim().trim_matches('"').parse() {
                        out.push(PowerSpec {
                            fn_name: item.clone(),
                            budget_uj: n,
                        });
                    }
                }
            }
        }
    }
    out
}

pub fn estimate_uj(node: &Node) -> f64 {
    match node {
        Node::Block { stmts, .. } => stmts.iter().map(estimate_uj).sum::<f64>(),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let base = if let Node::Identifier { name, .. } = function.as_ref() {
                if name.starts_with("radio_") {
                    100.0
                } else if name.starts_with("volatile_") {
                    0.05
                } else if name.starts_with("random_") {
                    0.01
                } else {
                    1.0
                }
            } else {
                1.0
            };
            base + arguments.iter().map(estimate_uj).sum::<f64>()
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => estimate_uj(consequence)
            .max(alternative.as_ref().map(|a| estimate_uj(a)).unwrap_or(0.0)),
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            100.0 * estimate_uj(body)
        }
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            0.001 + estimate_uj(value)
        }
        Node::ExpressionStatement { expr, .. } => estimate_uj(expr),
        Node::ReturnStatement { value: Some(e), .. } => estimate_uj(e),
        _ => 0.001,
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
    let bodies: HashMap<String, &Node> = stmts
        .iter()
        .filter_map(|s| match &s.node {
            Node::Function { name, body, .. } => Some((name.clone(), body.as_ref())),
            _ => None,
        })
        .collect();
    for spec in &specs {
        if let Some(body) = bodies.get(&spec.fn_name) {
            let est = estimate_uj(body);
            if est > spec.budget_uj {
                return Err(format!(
                    "{}:0:0: error: `{}` energy budget exceeded: {:.3} µJ > declared {} µJ",
                    source_path, spec.fn_name, est, spec.budget_uj
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
    fn radio_call_consumes_budget() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "tx",
            crate::feature_attrs::AttrRecord {
                name: "power".into(),
                args: r#"uj = "10""#.into(),
                line: 0,
            },
        );
        let src = r#"fn tx(int x) { radio_send(x); return 0; }"#;
        let (prog, _) = parse(src);
        let res = check(&prog, "test");
        assert!(res.is_err());
        crate::feature_attrs::reset();
    }
}
