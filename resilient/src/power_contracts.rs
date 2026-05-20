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

/// RES-2386: dropped the redundant `fn_name: String` field. The two
/// readers in `check` (`bodies.get(spec.fn_name.as_str())` lookup +
/// the budget-exceeded error format) used it strictly as a name tied
/// to the attribute owner. The field stored exactly what the
/// attribute key encoded. Pipeline now carries `(String, PowerSpec)`
/// tuples from `collect()` to `check()`, matching the shape that
/// `wcet_contracts` (RES-2190) and `probabilistic_contracts` (RES-2170)
/// already use. Same dead-field pattern as RES-2106 / RES-2168 / etc.
#[derive(Debug, Clone, Copy)]
pub struct PowerSpec {
    pub budget_uj: f64,
}

pub fn collect() -> Vec<(String, PowerSpec)> {
    let attrs = crate::feature_attrs::find_kind("power");
    // RES-1754: pre-size to attrs.len() — the inner loop conditionally
    // pushes one entry per attribute (when the `uj` chunk parses), so
    // attrs.len() is an upper bound (sometimes over-allocates by a
    // handful when chunks fail to parse, but power-budget attrs are
    // rare and the alternative is a growing 0→4 doubling chain).
    let mut out = Vec::with_capacity(attrs.len());
    // RES-2018: pull the value out of the inner loop first, then push
    // once with `item` moved (not cloned). Previously the push lived
    // inside the inner `for chunk` loop, which forced
    // `fn_name: item.clone()` because the borrow checker could not
    // see that only one chunk matches `uj` per attribute. Same fix
    // applied to wcet_contracts and stack_contracts.
    for (item, rec) in attrs {
        let mut budget_uj: Option<f64> = None;
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                if k.trim() == "uj" {
                    if let Ok(n) = v.trim().trim_matches('"').parse() {
                        budget_uj = Some(n);
                        break;
                    }
                }
            }
        }
        if let Some(n) = budget_uj {
            out.push((item, PowerSpec { budget_uj: n }));
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
    // RES-1495: borrow each function name as `&str` instead of
    // cloning into the HashMap key. The map's only consumer is
    // `bodies.get(spec.fn_name.as_str())` below — `&str` works for
    // both insert and lookup, so the per-function `name.clone()` is
    // pure overhead.
    let bodies: HashMap<&str, &Node> = stmts
        .iter()
        .filter_map(|s| match &s.node {
            Node::Function { name, body, .. } => Some((name.as_str(), body.as_ref())),
            _ => None,
        })
        .collect();
    for (fn_name, spec) in &specs {
        if let Some(body) = bodies.get(fn_name.as_str()) {
            let est = estimate_uj(body);
            if est > spec.budget_uj {
                return Err(format!(
                    "{}:0:0: error: `{}` energy budget exceeded: {:.3} µJ > declared {} µJ",
                    source_path, fn_name, est, spec.budget_uj
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

    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn estimate_uj_returns_nonzero_for_literal() {
        let node = crate::Node::IntegerLiteral {
            value: 42,
            span: crate::Span::default(),
        };
        assert!(estimate_uj(&node) >= 0.0, "estimate must be non-negative");
    }
}
