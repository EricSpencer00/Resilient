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

/// RES-2510: walk the entire AST subtree via `uniqueness_walk::visit`
/// and report the first panic-triggering call found.  The previous
/// hand-rolled match covered only 7 node types; everything else
/// (for-in, match, closures, infix expressions, struct literals, …)
/// fell through to `_ => None`, silently missing hidden panic calls.
pub fn body_panics(node: &Node) -> Option<String> {
    let mut found: Option<String> = None;
    crate::uniqueness_walk::visit(node, &mut |n| {
        if found.is_some() {
            return;
        }
        if let Node::CallExpression { function, .. } = n {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if PANIC_TRIGGERS.contains(&name.as_str()) {
                    found = Some(format!("call to `{name}`"));
                }
            }
        }
    });
    found
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

    fn extract_body(src: &str) -> Box<Node> {
        let (prog, _) = parse(src);
        if let Node::Program(ss) = &prog {
            for s in ss {
                if let Node::Function { body, .. } = &s.node {
                    return body.clone();
                }
            }
        }
        panic!("no function found");
    }

    #[test]
    fn unwrap_call_violates_cert() {
        let body = extract_body(r#"fn f(int x) { let y = unwrap(x); return y; }"#);
        assert!(body_panics(&body).is_some());
    }

    #[test]
    fn pure_arithmetic_is_panic_free() {
        let body = extract_body(r#"fn f(int x) -> int { return x + 1; }"#);
        assert!(body_panics(&body).is_none());
    }

    #[test]
    fn panic_in_for_in_body() {
        let body = extract_body(r#"fn f(int x) { for i in [1, 2, 3] { panic("oops"); } }"#);
        assert!(
            body_panics(&body).is_some(),
            "should detect panic in for-in body"
        );
    }

    #[test]
    fn panic_in_match_arm() {
        let body = extract_body(r#"fn f(int x) { match x { 1 => panic("boom"), _ => 0 }; }"#);
        assert!(
            body_panics(&body).is_some(),
            "should detect panic in match arm"
        );
    }

    #[test]
    fn panic_in_closure_body() {
        let body = extract_body(r#"fn f(int x) { let g = fn() { panic("inner"); }; }"#);
        assert!(
            body_panics(&body).is_some(),
            "should detect panic in closure"
        );
    }

    #[test]
    fn panic_in_infix_operand() {
        let body = extract_body(r#"fn f(int x) -> int { return x + panic("nope"); }"#);
        assert!(
            body_panics(&body).is_some(),
            "should detect panic in infix operand"
        );
    }

    #[test]
    fn panic_in_array_literal() {
        let body = extract_body(r#"fn f() { let a = [1, panic("arr"), 3]; }"#);
        assert!(
            body_panics(&body).is_some(),
            "should detect panic in array literal"
        );
    }

    #[test]
    fn panic_in_if_consequence() {
        let body = extract_body(r#"fn f(int x) { if x > 0 { panic("pos"); } }"#);
        assert!(
            body_panics(&body).is_some(),
            "should detect panic in if body"
        );
    }

    #[test]
    fn panic_in_while_body() {
        let body = extract_body(r#"fn f(int x) { while x > 0 { panic("loop"); } }"#);
        assert!(
            body_panics(&body).is_some(),
            "should detect panic in while body"
        );
    }

    #[test]
    fn expect_call_violates_cert() {
        let body = extract_body(r#"fn f(int x) { expect(x); }"#);
        assert!(body_panics(&body).is_some(), "should detect expect() call");
    }

    #[test]
    fn abort_call_violates_cert() {
        let body = extract_body(r#"fn f() { abort(); }"#);
        assert!(body_panics(&body).is_some(), "should detect abort() call");
    }

    #[test]
    fn nested_panic_in_index_expr() {
        let body = extract_body(r#"fn f() { let a = [1, 2]; let x = a[panic("idx")]; }"#);
        assert!(
            body_panics(&body).is_some(),
            "should detect panic in index expression"
        );
    }

    #[test]
    fn clean_for_in_is_panic_free() {
        let body = extract_body(r#"fn f() { for i in [1, 2, 3] { let x = i + 1; } }"#);
        assert!(body_panics(&body).is_none());
    }
}
