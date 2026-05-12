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
    //
    // RES-1527: single-pass partition into `secret_fns` / `public_fns`
    // instead of two filter walks over `labels`. The historic shape
    // iterated `labels` twice with a filter+map per pass; this one walk
    // dispatches each entry to its set in one match. Marginal on small
    // registries but cleaner and reads better.
    let mut secret_fns: HashSet<&str> = HashSet::new();
    let mut public_fns: HashSet<&str> = HashSet::new();
    for (k, l) in &labels {
        match l {
            Label::Secret => {
                secret_fns.insert(k.as_str());
            }
            Label::Public => {
                public_fns.insert(k.as_str());
            }
            _ => {}
        }
    }

    // RES-1527: skip the program walk when no `#[public]` function
    // exists. The leak check only fires inside a `public_fns`-named
    // function body, so without any public sink there is no possible
    // diagnostic — and `walk_calls` produces nothing useful. The
    // overwhelming majority of programs that ship `#[secret]` /
    // `#[public]` attributes only tag a couple of fns with one or the
    // other (not both); whichever side is empty, the walk is dead
    // work. Same shape as the `crate::secret_erasure::check`
    // empty-set fast-reject.
    if public_fns.is_empty() {
        return errors;
    }

    let Node::Program(stmts) = program else {
        return errors;
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            if public_fns.contains(name.as_str()) {
                // walk body: any call to a secret-tagged fn is a leak.
                let mut leaks: Vec<&str> = Vec::new();
                walk_calls(body, &mut leaks);
                for callee in leaks {
                    if secret_fns.contains(callee) {
                        errors.push(format!(
                            "info-flow: `{}` is `#[public]` but transitively calls `#[secret]` fn `{}`",
                            name, callee
                        ));
                    }
                }
            }
        }
    }
    errors
}

fn walk_calls<'a>(node: &'a Node, out: &mut Vec<&'a str>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                out.push(name.as_str());
            }
            for a in arguments {
                walk_calls(a, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_calls(s, out);
            }
        }
        Node::ReturnStatement { value: Some(e), .. } => walk_calls(e, out),
        Node::LetStatement { value, .. } => walk_calls(value, out),
        Node::ExpressionStatement { expr, .. } => walk_calls(expr, out),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_calls(condition, out);
            walk_calls(consequence, out);
            if let Some(e) = alternative {
                walk_calls(e, out);
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
}
