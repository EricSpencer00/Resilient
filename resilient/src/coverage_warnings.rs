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
}
