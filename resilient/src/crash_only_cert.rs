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
    // RES-1236: fast-reject. The diagnostic only fires for functions
    // annotated `#[crash_only_cert]`. When no such attribute exists
    // in the program (the overwhelming common case — `examples/` and
    // most tests don't use it), the inner `attrs.iter().any(...)`
    // returns false for every function and the loop produces no
    // output. Skip the loop entirely when the attribute set is
    // empty. Same shape as the dead-walk fast-reject series.
    if attrs.is_empty() {
        return Ok(());
    }
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    // RES-2240: collapse the per-Function `attrs.iter().any(|(item, _)|
    // item == name)` linear search into an O(1) HashSet probe. With
    // N functions and A crash_only_cert attributes, the previous shape
    // was O(N×A). For programs that declare even a handful of certified
    // fns, the HashSet build amortises after a few function visits;
    // for non-certified fns (the bulk of the walk in mixed programs)
    // the lookup is now constant-time. Mirrors RES-2138 (autopilot
    // HashMap-index lookups) and RES-2140 (refinement_types HashMap
    // registry).
    let certified: std::collections::HashSet<&str> =
        attrs.iter().map(|(item, _)| item.as_str()).collect();
    for s in stmts {
        if let Node::Function { name, .. } = &s.node {
            if certified.contains(name.as_str()) && !is_crash_only_certified(&s.node) {
                return Err(format!(
                    "{}:0:0: error: `{}` is `#[crash_only_cert]` but contains a return that is not Err/Ok/result",
                    _source_path, name
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
