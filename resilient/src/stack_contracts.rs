//! Feature 29/50 — Stack Depth Contracts.
//!
//! `#[stack(bytes = 256)]` declares the maximum stack a fn may use.
//! Estimation is the call-graph maximum depth times an average
//! frame size (64 bytes/frame initial slice). The runtime uses the
//! attribute to size dedicated ISR stacks (downstream PR).

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct StackSpec {
    pub fn_name: String,
    pub budget_bytes: u64,
}

const FRAME_BYTES: u64 = 64;

pub fn collect() -> Vec<StackSpec> {
    let attrs = crate::feature_attrs::find_kind("stack");
    // RES-1784: pre-size to attrs.len() — conditional push (only when
    // `bytes` chunk parses), so this is an upper bound.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                if k.trim() == "bytes" {
                    if let Ok(n) = v.trim().trim_matches('"').parse() {
                        out.push(StackSpec {
                            fn_name: item.clone(),
                            budget_bytes: n,
                        });
                    }
                }
            }
        }
    }
    out
}

fn max_call_depth(node: &Node, current: u64) -> u64 {
    match node {
        Node::Block { stmts, .. } => stmts
            .iter()
            .map(|s| max_call_depth(s, current))
            .max()
            .unwrap_or(current),
        Node::CallExpression { arguments, .. } => {
            let arg_max = arguments
                .iter()
                .map(|a| max_call_depth(a, current))
                .max()
                .unwrap_or(current);
            arg_max + 1
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            let c = max_call_depth(consequence, current);
            let a = alternative
                .as_ref()
                .map(|a| max_call_depth(a, current))
                .unwrap_or(current);
            c.max(a)
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            max_call_depth(body, current)
        }
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            max_call_depth(value, current)
        }
        Node::ExpressionStatement { expr, .. } => max_call_depth(expr, current),
        Node::ReturnStatement { value: Some(e), .. } => max_call_depth(e, current),
        _ => current,
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
    // `power_contracts` / `wcet_contracts` in this PR.
    let bodies: HashMap<&str, &Node> = stmts
        .iter()
        .filter_map(|s| match &s.node {
            Node::Function { name, body, .. } => Some((name.as_str(), body.as_ref())),
            _ => None,
        })
        .collect();
    for spec in &specs {
        if let Some(body) = bodies.get(spec.fn_name.as_str()) {
            let depth = max_call_depth(body, 1);
            let bytes = depth * FRAME_BYTES;
            if bytes > spec.budget_bytes {
                return Err(format!(
                    "{}:0:0: error: `{}` stack budget exceeded: estimated {} bytes (depth {}) > declared {} bytes",
                    source_path, spec.fn_name, bytes, depth, spec.budget_bytes
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
    fn shallow_function_passes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "small",
            crate::feature_attrs::AttrRecord {
                name: "stack".into(),
                args: r#"bytes = "256""#.into(),
                line: 0,
            },
        );
        let src = r#"fn small(int x) { return x; }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn no_attribute_skips_check() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        // No stack attribute registered — even a deeply nested function passes.
        let src = "fn deep(int x) { return deep(deep(deep(x))); }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn budget_exceeded_returns_error() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        // Budget: 64 bytes (= 1 frame). Any call nests at least 2 frames.
        crate::feature_attrs::record(
            "tight",
            crate::feature_attrs::AttrRecord {
                name: "stack".into(),
                args: r#"bytes = "64""#.into(),
                line: 0,
            },
        );
        // Two nested calls → depth 2 → 128 bytes > 64 budget.
        let src = "fn helper(int x) { return x; }\nfn tight(int x) { return helper(helper(x)); }\n";
        let (prog, _) = parse(src);
        let result = check(&prog, "test");
        assert!(result.is_err(), "expected budget-exceeded error, got Ok");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("tight") && msg.contains("exceeded"),
            "error message should mention function name and 'exceeded': {msg}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn unknown_function_name_is_silent() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "missing_fn",
            crate::feature_attrs::AttrRecord {
                name: "stack".into(),
                args: r#"bytes = "64""#.into(),
                line: 0,
            },
        );
        // The function `missing_fn` is not defined in the source.
        let src = "fn other(int x) { return x; }\n";
        let (prog, _) = parse(src);
        // Should not error — function body not found means no depth estimate.
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }
}
