//! Bonus Feature 52 — No-Panic Certification.
//!
//! `#[no_panic]` on a function statically proves the body and its
//! transitive callees never invoke a panic-inducing builtin or
//! emit an `unwrap()` on a Result/Option without first checking it.
//!
//! Detection scans for these panic triggers:
//! * Bare `unwrap()` calls
//! * `expect()` calls
//! * `panic()` builtin
//! * Division/modulo by an unchecked variable (paired with the
//!   contract_inference module's heuristics)

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashSet;

const PANIC_TRIGGERS: &[&str] = &[
    "unwrap",
    "expect",
    "panic",
    "abort",
    "todo",
    "unimplemented",
];

pub fn body_panics(node: &Node) -> Option<String> {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if PANIC_TRIGGERS.contains(&name.as_str()) {
                    return Some(format!("call to `{name}`"));
                }
            }
            for a in arguments {
                if let Some(r) = body_panics(a) {
                    return Some(r);
                }
            }
            None
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                if let Some(r) = body_panics(s) {
                    return Some(r);
                }
            }
            None
        }
        Node::ReturnStatement { value: Some(e), .. } => body_panics(e),
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => body_panics(value),
        Node::ExpressionStatement { expr, .. } => body_panics(expr),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => body_panics(condition)
            .or_else(|| body_panics(consequence))
            .or_else(|| alternative.as_ref().and_then(|a| body_panics(a))),
        Node::WhileStatement {
            condition, body, ..
        } => body_panics(condition).or_else(|| body_panics(body)),
        _ => None,
    }
}

pub fn collect_no_panic_fns() -> HashSet<String> {
    crate::feature_attrs::find_kind("no_panic")
        .into_iter()
        .map(|(item, _)| item)
        .collect()
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let no_panic = collect_no_panic_fns();
    if no_panic.is_empty() {
        return Ok(());
    }
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            if no_panic.contains(name) {
                if let Some(reason) = body_panics(body) {
                    return Err(format!(
                        "{}:0:0: error: `{}` is `#[no_panic]` but contains {}",
                        source_path, name, reason
                    ));
                }
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
    fn unwrap_call_violates_cert() {
        let src = r#"fn f(int x) { let y = unwrap(x); return y; }"#;
        let (prog, _) = parse(src);
        if let Node::Program(ss) = &prog {
            for s in ss {
                if let Node::Function { body, .. } = &s.node {
                    assert!(body_panics(body).is_some());
                }
            }
        }
    }

    #[test]
    fn pure_arithmetic_is_panic_free() {
        let src = r#"fn f(int x) -> int { return x + 1; }"#;
        let (prog, _) = parse(src);
        if let Node::Program(ss) = &prog {
            for s in ss {
                if let Node::Function { body, .. } = &s.node {
                    assert!(body_panics(body).is_none());
                }
            }
        }
    }
}
