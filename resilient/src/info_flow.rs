//! Feature 16/50 — Information Flow / Non-Interference Types.
//!
//! `#[secret]` marks a value or parameter as classified; `#[public]`
//! marks one as observable. The compiler enforces that no `#[secret]`
//! value reaches a `#[public]` sink without going through a
//! declassification function tagged `#[declassify]`.
//!
//! This first slice tracks the secret/public classification on
//! parameters and return types, builds the data-flow approximation
//! at call sites, and reports any direct secret→public propagation.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Label {
    Secret,
    Public,
    Unknown,
}

pub fn collect_param_labels() -> HashMap<String, Label> {
    let mut out = HashMap::new();
    for (item, _rec) in crate::feature_attrs::find_kind("secret") {
        out.insert(item, Label::Secret);
    }
    for (item, _rec) in crate::feature_attrs::find_kind("public") {
        out.insert(item, Label::Public);
    }
    out
}

pub fn check_program(program: &Node) -> Vec<String> {
    let labels = collect_param_labels();
    let mut errors = Vec::new();
    if labels.is_empty() {
        return errors;
    }
    // RES-1439: store `&str` borrows into the `labels` map rather
    // than `&String` references. Then `walk_calls` can collect
    // `&str` borrows of identifier names (skipping the per-callee
    // `String::clone` it used to do), and the set membership check
    // `secret_fns.contains(callee)` works directly on `&str`.
    let secret_fns: HashSet<&str> = labels
        .iter()
        .filter(|(_, l)| **l == Label::Secret)
        .map(|(k, _)| k.as_str())
        .collect();
    let public_fns: HashSet<&str> = labels
        .iter()
        .filter(|(_, l)| **l == Label::Public)
        .map(|(k, _)| k.as_str())
        .collect();

    // RES-1527: skip the program walk when no public fn exists — the
    // leak diagnostic only fires inside public-tagged fn bodies, so
    // without any public fn the walk produces nothing.
    if public_fns.is_empty() {
        return errors;
    }

    let Node::Program(stmts) = program else {
        return errors;
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            if public_fns.contains(name.as_str()) {
                // RES-2282: walk the body once, pushing only callees
                // that match `secret_fns`. The previous shape collected
                // every CallExpression's callee identifier into a
                // `Vec<&str>` and then filtered against `secret_fns`
                // — for programs where most calls are to non-secret
                // fns (the common case), most pushes were dead
                // weight. Filtering at push time also lets us drop
                // the post-walk iteration entirely.
                let mut leaks: Vec<&str> = Vec::new();
                walk_calls(body, &secret_fns, &mut leaks);
                for callee in leaks {
                    errors.push(format!(
                        "info-flow: `{}` is `#[public]` but transitively calls `#[secret]` fn `{}`",
                        name, callee
                    ));
                }
            }
        }
    }
    errors
}

fn walk_calls<'a>(node: &'a Node, secret_fns: &HashSet<&str>, out: &mut Vec<&'a str>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                let callee = name.as_str();
                if secret_fns.contains(callee) {
                    out.push(callee);
                }
            }
            for a in arguments {
                walk_calls(a, secret_fns, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_calls(s, secret_fns, out);
            }
        }
        Node::ReturnStatement { value: Some(e), .. } => walk_calls(e, secret_fns, out),
        Node::LetStatement { value, .. } => walk_calls(value, secret_fns, out),
        Node::ExpressionStatement { expr, .. } => walk_calls(expr, secret_fns, out),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_calls(condition, secret_fns, out);
            walk_calls(consequence, secret_fns, out);
            if let Some(e) = alternative {
                walk_calls(e, secret_fns, out);
            }
        }
        _ => {}
    }
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let errs = check_program(program);
    if !errs.is_empty() {
        return Err(format!("{}:0:0: error: {}", source_path, errs[0]));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn secret_to_public_is_blocked() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "leak",
            crate::feature_attrs::AttrRecord {
                name: "secret".into(),
                args: String::new(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "log",
            crate::feature_attrs::AttrRecord {
                name: "public".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"
            fn leak(int x) -> int { return x; }
            fn log(int x) -> int { return leak(x); }
        "#;
        let (prog, _) = parse(src);
        assert!(!check_program(&prog).is_empty());
        crate::feature_attrs::reset();
    }
    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_program_no_attrs_returns_empty() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check_program(&prog).is_empty());
        crate::feature_attrs::reset();
    }
}
