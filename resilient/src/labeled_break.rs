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
        Node::Break { .. } | Node::Continue { .. } if depth >= 3 => {
            out.push(DeepBreakWarning {
                function: fn_name.to_string(),
                depth,
            });
        }
        Node::Break { .. } | Node::Continue { .. } => {}
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

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // Fast-reject: skip programs with no loops at all.
    let has_loop = crate::uniqueness_walk::any_node(program, |n| {
        matches!(n, Node::WhileStatement { .. } | Node::ForInStatement { .. })
    });
    if !has_loop {
        return Ok(());
    }
    let warnings = analyze(program);
    for w in &warnings {
        eprintln!(
            "warning: `{}` has a break/continue nested {} loop(s) deep — \
             consider refactoring with labeled break once the syntax is available",
            w.function, w.depth
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn empty_program_no_warnings() {
        let p = Node::Program(vec![]);
        assert!(analyze(&p).is_empty());
    }

    #[test]
    fn pure_function_no_warnings() {
        let src = "fn f(int x) -> int { return x + 1; }\n";
        let (prog, _) = parse(src);
        assert!(analyze(&prog).is_empty());
    }

    #[test]
    fn check_ok_for_any_program() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_ok_with_single_loop() {
        let src = r#"fn f(IntArr xs) { for x in xs { if x > 0 { break; } } }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn deeply_nested_break_detected_by_walk() {
        // walk() is the recursive function — test it directly.
        // Three levels of nesting: outer (depth 1) → mid (depth 2) → inner (depth 3).
        // A break at depth 3 must produce a warning.
        use crate::span::Span;
        let inner_break = Node::Block {
            stmts: vec![Node::Break { span: Span::default() }],
            span: Span::default(),
        };
        let mid = Node::ForInStatement {
            name: "c".into(),
            iterable: Box::new(Node::Identifier { name: "xs".into(), span: Span::default() }),
            body: Box::new(inner_break),
            invariants: vec![],
            span: Span::default(),
        };
        let outer2 = Node::ForInStatement {
            name: "b".into(),
            iterable: Box::new(Node::Identifier { name: "xs".into(), span: Span::default() }),
            body: Box::new(mid),
            invariants: vec![],
            span: Span::default(),
        };
        let outer1 = Node::ForInStatement {
            name: "a".into(),
            iterable: Box::new(Node::Identifier { name: "xs".into(), span: Span::default() }),
            body: Box::new(outer2),
            invariants: vec![],
            span: Span::default(),
        };
        let mut warnings = Vec::new();
        walk(&outer1, "test_fn", 0, &mut warnings);
        assert!(
            !warnings.is_empty(),
            "three-deep nested break must be flagged"
        );
        assert!(warnings[0].depth >= 3);
    }
}
