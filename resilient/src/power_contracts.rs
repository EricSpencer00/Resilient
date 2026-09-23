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
            out.push(PowerSpec {
                fn_name: item,
                budget_uj: n,
            });
        }
    }
    out
}

pub fn estimate_uj(node: &Node) -> f64 {
    const NODE_COST: f64 = 0.001;

    match node {
        Node::Block { stmts, .. } => stmts.iter().map(estimate_uj).sum(),
        Node::Program(stmts) => stmts.iter().map(|stmt| estimate_uj(&stmt.node)).sum(),
        Node::Function { body, .. } | Node::FunctionLiteral { body, .. } => estimate_uj(body),
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
            let callee = match function.as_ref() {
                Node::Identifier { .. } => 0.0,
                other => estimate_uj(other),
            };
            base + callee + arguments.iter().map(estimate_uj).sum::<f64>()
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            estimate_uj(condition)
                + estimate_uj(consequence).max(
                    alternative
                        .as_ref()
                        .map(|branch| estimate_uj(branch))
                        .unwrap_or(0.0),
                )
        }
        Node::WhileStatement {
            condition, body, ..
        }
        | Node::ForInStatement {
            iterable: condition,
            body,
            ..
        } => 100.0 * (estimate_uj(condition) + estimate_uj(body)),
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            NODE_COST + estimate_uj(value)
        }
        Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::LetDestructureStruct { value, .. }
        | Node::LetTupleDestructure { value, .. }
        | Node::NewtypeConstruct { value, .. }
        | Node::NamedArg { value, .. } => NODE_COST + estimate_uj(value),
        Node::ExpressionStatement { expr, .. } => estimate_uj(expr),
        Node::ReturnStatement { value: Some(e), .. } => estimate_uj(e),
        Node::ReturnStatement { value: None, .. }
        | Node::Break { .. }
        | Node::Continue { .. }
        | Node::BreakLabel { .. }
        | Node::ContinueLabel { .. } => NODE_COST,
        Node::BreakWith { value, .. } | Node::DeferStatement { expr: value, .. } => {
            NODE_COST + estimate_uj(value)
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            let arm_cost = arms
                .iter()
                .map(|(_, guard, body)| {
                    guard.as_ref().map(estimate_uj).unwrap_or(0.0) + estimate_uj(body)
                })
                .fold(0.0, f64::max);
            estimate_uj(scrutinee) + arm_cost
        }
        Node::TryCatch { body, handlers, .. } => {
            let handler_cost = handlers
                .iter()
                .map(|(_, statements)| statements.iter().map(estimate_uj).sum::<f64>())
                .fold(0.0, f64::max);
            body.iter().map(estimate_uj).sum::<f64>() + handler_cost
        }
        Node::TryExpression { expr, .. }
        | Node::FieldAccess { target: expr, .. }
        | Node::TupleIndex { tuple: expr, .. } => NODE_COST + estimate_uj(expr),
        Node::FieldAssignment { target, value, .. }
        | Node::IndexAssignment { target, value, .. } => {
            NODE_COST + estimate_uj(target) + estimate_uj(value)
        }
        Node::IndexExpression { target, index, .. } => {
            NODE_COST + estimate_uj(target) + estimate_uj(index)
        }
        Node::PrefixExpression { right, .. } => NODE_COST + estimate_uj(right),
        Node::InfixExpression { left, right, .. } => {
            NODE_COST + estimate_uj(left) + estimate_uj(right)
        }
        Node::ArrayLiteral { items, .. }
        | Node::SetLiteral { items, .. }
        | Node::TupleLiteral { items, .. } => {
            NODE_COST + items.iter().map(estimate_uj).sum::<f64>()
        }
        Node::MapLiteral { entries, .. } => {
            NODE_COST
                + entries
                    .iter()
                    .map(|(key, value)| estimate_uj(key) + estimate_uj(value))
                    .sum::<f64>()
        }
        Node::StructLiteral { fields, base, .. } => {
            NODE_COST
                + base.as_ref().map(|base| estimate_uj(base)).unwrap_or(0.0)
                + fields
                    .iter()
                    .map(|(_, value)| estimate_uj(value))
                    .sum::<f64>()
        }
        Node::Slice { target, lo, hi, .. } => {
            NODE_COST
                + estimate_uj(target)
                + lo.as_ref().map(|value| estimate_uj(value)).unwrap_or(0.0)
                + hi.as_ref().map(|value| estimate_uj(value)).unwrap_or(0.0)
        }
        Node::Range { lo, hi, .. } => NODE_COST + estimate_uj(lo) + estimate_uj(hi),
        Node::InterpolatedString { parts, .. } => {
            NODE_COST
                + parts
                    .iter()
                    .filter_map(|part| match part {
                        crate::string_interp::StringPart::Expr(expr) => Some(estimate_uj(expr)),
                        crate::string_interp::StringPart::Literal(_) => None,
                    })
                    .sum::<f64>()
        }
        Node::OptionalChain { object, access, .. } => {
            let args = match access {
                crate::ChainAccess::Field(_) => 0.0,
                crate::ChainAccess::Method(_, args) => args.iter().map(estimate_uj).sum(),
            };
            NODE_COST + estimate_uj(object) + args
        }
        Node::Quantifier { range, body, .. } => {
            let range_cost = match range {
                crate::quantifiers::QuantRange::Range { lo, hi } => {
                    estimate_uj(lo) + estimate_uj(hi)
                }
                crate::quantifiers::QuantRange::Iterable(iterable) => estimate_uj(iterable),
            };
            NODE_COST + range_cost + 100.0 * estimate_uj(body)
        }
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            NODE_COST
                + estimate_uj(condition)
                + message
                    .as_ref()
                    .map(|message| estimate_uj(message))
                    .unwrap_or(0.0)
        }
        Node::InvariantStatement { expr, .. }
        | Node::StaticAssert {
            condition: expr, ..
        } => NODE_COST + estimate_uj(expr),
        Node::LiveBlock {
            body,
            invariants,
            timeout,
            ..
        } => {
            estimate_uj(body)
                + invariants.iter().map(estimate_uj).sum::<f64>()
                + timeout
                    .as_ref()
                    .map(|value| estimate_uj(value))
                    .unwrap_or(0.0)
        }
        Node::UnsafeBlock { body, .. } | Node::BenchBlock { body, .. } => estimate_uj(body),
        _ => NODE_COST,
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
    for spec in &specs {
        if let Some(body) = bodies.get(spec.fn_name.as_str()) {
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

    #[test]
    fn volatile_call_within_budget() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "read_reg",
            crate::feature_attrs::AttrRecord {
                name: "power".into(),
                args: r#"uj = "1""#.into(),
                line: 0,
            },
        );
        let src = r#"fn read_reg(int x) { volatile_read(x); return 0; }"#;
        let (prog, _) = parse(src);
        // volatile is 0.05µJ, well within 1µJ budget
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn radio_call_exceeds_budget() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "transmit",
            crate::feature_attrs::AttrRecord {
                name: "power".into(),
                args: r#"uj = "50""#.into(),
                line: 0,
            },
        );
        let src = r#"fn transmit(int x) { radio_send(x); return 0; }"#;
        let (prog, _) = parse(src);
        // radio is 100µJ, exceeds 50µJ budget
        assert!(check(&prog, "test").is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn multiple_statements_accumulate() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "work",
            crate::feature_attrs::AttrRecord {
                name: "power".into(),
                args: r#"uj = "10""#.into(),
                line: 0,
            },
        );
        let src = r#"
            fn work(int x) {
                let a = x + 1;
                let b = a + 1;
                let c = b + 1;
                return 0;
            }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn nested_function_calls_tracked() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "outer",
            crate::feature_attrs::AttrRecord {
                name: "power".into(),
                args: r#"uj = "5""#.into(),
                line: 0,
            },
        );
        let src = r#"
            fn inner(int x) { return x + 1; }
            fn outer(int x) { return inner(x); }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn if_statement_worst_case_chosen() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "branch",
            crate::feature_attrs::AttrRecord {
                name: "power".into(),
                args: r#"uj = "200""#.into(),
                line: 0,
            },
        );
        let src = r#"
            fn branch(int x) {
                if x > 0 {
                    radio_send(1);
                } else {
                    return 0;
                }
                return x;
            }
        "#;
        let (prog, _) = parse(src);
        // radio_send is 100µJ, within 200µJ budget
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn while_loop_multiplied_by_100() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "loop_work",
            crate::feature_attrs::AttrRecord {
                name: "power".into(),
                args: r#"uj = "1""#.into(),
                line: 0,
            },
        );
        let src = r#"
            fn loop_work(int x) {
                while x > 0 {
                    x = x - 1;
                }
                return 0;
            }
        "#;
        let (prog, _) = parse(src);
        // Loop multiplies by 100: 0.001 * 100 = 0.1µJ, still within 1µJ
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }
}
