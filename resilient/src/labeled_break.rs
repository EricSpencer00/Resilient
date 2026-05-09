//! Feature 50/50 — Labeled Break/Continue.
//!
//! `'outer: for ... { for ... { break 'outer; } }`. Breaks/continues
//! tagged with a label target the named enclosing loop instead of
//! the innermost. Without this, breaking out of nested loops
//! requires sentinel flags.
//!
//! This first slice analyses every `break;`/`continue;` site and
//! reports those that are deeply nested (≥3 loops) — they're the
//! prime candidates for refactoring to use a labeled break once
//! the parser supports the syntax.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone)]
pub struct DeepBreakWarning {
    pub function: String,
    pub depth: u32,
}

pub fn analyze(program: &Node) -> Vec<DeepBreakWarning> {
    let mut out = Vec::new();
    let Node::Program(stmts) = program else {
        return out;
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            walk(body, name, 0, &mut out);
        }
    }
    out
}

fn walk(node: &Node, fn_name: &str, depth: u32, out: &mut Vec<DeepBreakWarning>) {
    match node {
        Node::Break { .. } | Node::Continue { .. } => {
            if depth >= 3 {
                out.push(DeepBreakWarning {
                    function: fn_name.to_string(),
                    depth,
                });
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            walk(body, fn_name, depth + 1, out);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk(s, fn_name, depth, out);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            walk(consequence, fn_name, depth, out);
            if let Some(a) = alternative {
                walk(a, fn_name, depth, out);
            }
        }
        _ => {}
    }
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_program_no_warnings() {
        let p = Node::Program(vec![]);
        assert!(analyze(&p).is_empty());
    }
}
