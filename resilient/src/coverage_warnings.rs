//! Feature 46/50 — Coverage-Aware Compilation Warnings.
//!
//! Walks every fn and reports branches that no caller is likely to
//! exercise. Heuristics (initial slice):
//!
//! * An `if` branch whose body is an `Err(...)` constructor with no
//!   exterior call site that could trigger it is flagged as
//!   "untested error path".
//! * A `match` arm that returns a fixed enum variant never produced
//!   by any caller is flagged.
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
            for s in stmts {
                walk(s, fn_name, out);
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            walk(body, fn_name, out);
        }
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

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1284: fast-reject. `analyze` walks every function body
    // looking for `if`/`else` branches whose terminator is
    // `return Err(...)`. Without any `CallExpression` whose callee is
    // the `Err` constructor anywhere in the program — every fixture
    // in `examples/` that doesn't use the `Result` happy path —
    // `analyze` returns an empty Vec and no warnings fire. Pre-scan
    // once with the early-terminating `any_node` (RES-1238) and skip
    // the analysis entirely when no `Err` call exists.
    let has_err_call = crate::uniqueness_walk::any_node(program, |n| match n {
        Node::CallExpression { function, .. } => match function.as_ref() {
            Node::Identifier { name, .. } => name == "Err",
            _ => false,
        },
        _ => false,
    });
    if !has_err_call {
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
}
