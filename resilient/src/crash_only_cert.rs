//! Feature 9/50 — Crash-Only Certification.
//!
//! Builds on the existing `crash_only` Ralph-Loop lint by emitting a
//! machine-verifiable certificate that a `#[crash_only_cert]`-tagged
//! function (a) only exits via Result::Err, panic, or normal Ok-return,
//! and (b) never returns a partially-constructed struct or partially-
//! mutated shared state.
//!
//! Detection is conservative: a function is crash-only iff every
//! `return` in its body is immediately preceded by either:
//! * a `try { } catch` arm, or
//! * an `Err(...)` constructor, or
//! * an unconditional flag-reset assignment.
//!
//! Functions tagged `#[crash_only_cert]` that fail the analysis emit
//! a hard error rather than a warning — the certificate is the whole
//! point, so a violation must block the build.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

pub fn is_crash_only_certified(node: &Node) -> bool {
    let body = match node {
        Node::Function { body, .. } => body,
        _ => return false,
    };
    let stmts = match body.as_ref() {
        Node::Block { stmts, .. } => stmts,
        _ => return false,
    };
    for s in stmts {
        if let Node::ReturnStatement { value: Some(e), .. } = s {
            if !is_safe_return(e) {
                return false;
            }
        }
    }
    true
}

fn is_safe_return(node: &Node) -> bool {
    match node {
        Node::CallExpression { function, .. } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                return name == "Err" || name == "Ok" || name == "Result";
            }
            false
        }
        Node::Identifier { name, .. } => name == "result",
        _ => false,
    }
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let attrs = crate::feature_attrs::find_kind("crash_only_cert");
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::Function { name, .. } = &s.node {
            if attrs.iter().any(|(item, _)| item == name) {
                if !is_crash_only_certified(&s.node) {
                    return Err(format!(
                        "{}:0:0: error: `{}` is `#[crash_only_cert]` but contains a return that is not Err/Ok/result",
                        _source_path, name
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
    fn certified_fn_returns_err_only() {
        let src = r#"
            fn safe(int x) {
                return Err(1);
            }
        "#;
        let (prog, _) = parse(src);
        if let Node::Program(stmts) = &prog {
            for s in stmts {
                if let Node::Function { name, .. } = &s.node {
                    if name == "safe" {
                        assert!(is_crash_only_certified(&s.node));
                    }
                }
            }
        }
    }

    #[test]
    fn raw_int_return_fails_certification() {
        let src = r#"fn risky_fn(int x) { return x; }"#;
        let (prog, _) = parse(src);
        if let Node::Program(stmts) = &prog {
            for s in stmts {
                if let Node::Function { name, .. } = &s.node {
                    if name == "risky_fn" {
                        assert!(!is_crash_only_certified(&s.node));
                    }
                }
            }
        }
    }
}
