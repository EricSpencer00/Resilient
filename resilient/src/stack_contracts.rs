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
    let mut out = Vec::new();
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
    let bodies: HashMap<String, &Node> = stmts
        .iter()
        .filter_map(|s| match &s.node {
            Node::Function { name, body, .. } => Some((name.clone(), body.as_ref())),
            _ => None,
        })
        .collect();
    for spec in &specs {
        if let Some(body) = bodies.get(&spec.fn_name) {
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
}
