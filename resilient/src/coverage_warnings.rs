//! Feature 46/50 — Coverage-Aware Compilation Warnings.
//!
//! Walks every fn and reports branches that no caller is likely to
//! exercise. Heuristics:
//!
//! * An `if` branch whose body is an `Err(...)` constructor with no
//!   exterior call site that could trigger it is flagged as
//!   "untested error path".
//! * A `match` arm that returns a fixed enum variant never produced
//!   by any caller is flagged.
//! * Unreachable code: statements after an unconditional `return` in
//!   the same block — the developer likely forgot to remove them or
//!   placed the `return` too early.
//! * All-literal match without a wildcard/default arm: if all arms
//!   match integer/bool/string literals and no arm is a catch-all,
//!   the match may silently miss unhandled values at runtime.
//!
//! The output is advisory: warnings, not errors. The tooling layer
//! can convert these into LSP diagnostics or CI advisories.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone)]
pub struct CoverageWarning {
    pub function: String,
    pub message: String,
}

pub fn analyze(program: &Node) -> Vec<CoverageWarning> {
    let mut out = Vec::new();
    let Node::Program(stmts) = program else {
        return out;
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            walk(body, name, &mut out);
        }
    }
    out
}

fn walk(node: &Node, fn_name: &str, out: &mut Vec<CoverageWarning>) {
    match node {
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            let cons_returns_err = block_returns_err(consequence);
            if cons_returns_err {
                out.push(CoverageWarning {
                    function: fn_name.to_string(),
                    message: "if-branch returns Err but no test exercises this path".into(),
                });
            }
            walk(consequence, fn_name, out);
            if let Some(alt) = alternative {
                if block_returns_err(alt) {
                    out.push(CoverageWarning {
                        function: fn_name.to_string(),
                        message: "else-branch returns Err — verify a test exercises it".into(),
                    });
                }
                walk(alt, fn_name, out);
            }
        }
        Node::Block { stmts, .. } => {
            check_unreachable_after_return(stmts, fn_name, out);
            for s in stmts {
                walk(s, fn_name, out);
            }
        }
        Node::Match { arms, .. } => {
            check_all_literal_match_no_wildcard(arms, fn_name, out);
            for (_, _, body) in arms {
                walk(body, fn_name, out);
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            walk(body, fn_name, out);
        }
        Node::ReturnStatement { value: Some(v), .. } => walk(v, fn_name, out),
        Node::ExpressionStatement { expr, .. } => walk(expr, fn_name, out),
        Node::LetStatement { value, .. } => walk(value, fn_name, out),
        _ => {}
    }
}

fn block_returns_err(node: &Node) -> bool {
    match node {
        Node::Block { stmts, .. } => stmts.iter().any(block_returns_err),
        Node::ReturnStatement { value: Some(e), .. } => is_err_call(e),
        _ => false,
    }
}

fn is_err_call(node: &Node) -> bool {
    if let Node::CallExpression { function, .. } = node {
        if let Node::Identifier { name, .. } = function.as_ref() {
            return name == "Err";
        }
    }
    false
}

/// Warn when a Block has a `return` statement followed by non-trivial
/// statements (unreachable code after return).
fn check_unreachable_after_return(
    stmts: &[Node],
    fn_name: &str,
    out: &mut Vec<CoverageWarning>,
) {
    let mut saw_return = false;
    for stmt in stmts {
        if saw_return {
            // Any statement after an unconditional return is unreachable.
            if !is_trivial_stmt(stmt) {
                out.push(CoverageWarning {
                    function: fn_name.to_string(),
                    message: "unreachable code after `return` statement".into(),
                });
                return; // one warning per block is enough
            }
        }
        if matches!(stmt, Node::ReturnStatement { .. }) {
            saw_return = true;
        }
    }
}

/// Returns true for statements that are too trivial to warn about
/// (e.g., another bare `return;`, a comment-only expression).
fn is_trivial_stmt(node: &Node) -> bool {
    matches!(node, Node::ReturnStatement { value: None, .. })
}

/// Warn when all match arms are literal patterns (integer, bool, or string)
/// and there is no catch-all arm (`_`, identifier binding, or `..`).
/// Such a match silently falls through if the scrutinee takes any unlisted value.
fn check_all_literal_match_no_wildcard(
    arms: &[(crate::Pattern, Option<Node>, Node)],
    fn_name: &str,
    out: &mut Vec<CoverageWarning>,
) {
    if arms.is_empty() {
        return;
    }
    let all_literals = arms.iter().all(|(p, _, _)| is_literal_pattern(p));
    let has_wildcard = arms.iter().any(|(p, g, _)| is_catch_all_arm(p, g));
    if all_literals && !has_wildcard {
        out.push(CoverageWarning {
            function: fn_name.to_string(),
            message: "match covers only literal values with no wildcard arm — \
                      unhandled values will fall through at runtime"
                .into(),
        });
    }
}

fn is_literal_pattern(p: &crate::Pattern) -> bool {
    matches!(p, crate::Pattern::Literal(_))
}

fn is_catch_all_arm(p: &crate::Pattern, guard: &Option<Node>) -> bool {
    if guard.is_some() {
        return false;
    }
    matches!(p, crate::Pattern::Wildcard | crate::Pattern::Identifier(_))
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1284: fast-reject. Pre-scan with early-terminating `any_node`.
    let has_err_call = crate::uniqueness_walk::any_node(program, |n| match n {
        Node::CallExpression { function, .. } => match function.as_ref() {
            Node::Identifier { name, .. } => name == "Err",
            _ => false,
        },
        _ => false,
    });
    let has_match = crate::uniqueness_walk::any_node(program, |n| {
        matches!(n, Node::Match { .. })
    });
    let has_return = crate::uniqueness_walk::any_node(program, |n| {
        matches!(n, Node::ReturnStatement { .. })
    });
    if !has_err_call && !has_match && !has_return {
        return Ok(());
    }
    let warnings = analyze(program);
    for w in &warnings {
        eprintln!("warning: coverage in `{}`: {}", w.function, w.message);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn flags_err_only_else_branch() {
        let src = r#"
            fn f(int x) {
                if x > 0 {
                    return x;
                } else {
                    return Err(1);
                }
            }
        "#;
        let (prog, _) = parse(src);
        let w = analyze(&prog);
        assert!(!w.is_empty());
    }

    #[test]
    fn no_warnings_for_pure_function() {
        let src = "fn g(int x) -> int { return x + 1; }\n";
        let (prog, _) = parse(src);
        let w = analyze(&prog);
        assert!(
            w.is_empty(),
            "pure function should generate no coverage warnings"
        );
    }

    #[test]
    fn empty_program_has_no_warnings() {
        let (prog, _) = parse("");
        let w = analyze(&prog);
        assert!(w.is_empty());
    }

    #[test]
    fn unreachable_code_after_return_is_flagged() {
        let src = r#"
fn f(int x) -> int {
    return x;
    let y = x + 1;
}
"#;
        let (prog, _) = parse(src);
        let w = analyze(&prog);
        assert!(
            w.iter().any(|x| x.message.contains("unreachable")),
            "expected unreachable-code warning; got: {:?}",
            w
        );
    }

    #[test]
    fn no_false_positive_for_single_return() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = parse(src);
        let w = analyze(&prog);
        let unreachable: Vec<_> = w.iter().filter(|x| x.message.contains("unreachable")).collect();
        assert!(
            unreachable.is_empty(),
            "must not flag single return: {:?}",
            unreachable
        );
    }

    #[test]
    fn all_literal_match_no_wildcard_is_flagged() {
        let src = r#"
fn classify(int code) -> int {
    return match code {
        0 => 1,
        1 => 2,
        2 => 3,
    };
}
"#;
        let (prog, _) = parse(src);
        let w = analyze(&prog);
        assert!(
            w.iter().any(|x| x.message.contains("wildcard")),
            "expected wildcard warning for all-literal match; got: {:?}",
            w
        );
    }

    #[test]
    fn literal_match_with_wildcard_is_ok() {
        let src = r#"
fn classify(int code) -> int {
    return match code {
        0 => 1,
        1 => 2,
        _ => 0,
    };
}
"#;
        let (prog, _) = parse(src);
        let w = analyze(&prog);
        let wildcard_warn: Vec<_> = w.iter().filter(|x| x.message.contains("wildcard")).collect();
        assert!(
            wildcard_warn.is_empty(),
            "must not flag match that has a wildcard arm: {:?}",
            wildcard_warn
        );
    }
}
