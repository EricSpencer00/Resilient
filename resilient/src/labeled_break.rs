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
    use crate::{parse, run_program};

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── labeled break/continue integration tests ──────────────────────────────

    #[test]
    fn labeled_break_exits_outer_for_loop() {
        let r = run(
            r#"let found = 0;
outer: for i in 0..5 {
    for j in 0..5 {
        if i == 2 && j == 3 {
            found = i * 10 + j;
            break outer;
        }
    }
}
println(found);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("23"), "stdout: {}", r.stdout);
    }

    #[test]
    fn labeled_break_exits_outer_while_loop() {
        let r = run(
            r#"let i = 0;
let found = 0;
outer: while i < 5 {
    let j = 0;
    while j < 5 {
        if i == 1 && j == 2 {
            found = i * 10 + j;
            break outer;
        }
        j = j + 1;
    }
    i = i + 1;
}
println(found);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("12"), "stdout: {}", r.stdout);
    }

    #[test]
    fn labeled_continue_skips_outer_iteration() {
        // Collect values where inner loop doesn't fire `continue outer`
        let r = run(
            r#"let sum = 0;
outer: for i in 0..4 {
    for j in 0..3 {
        if j == 1 {
            continue outer;
        }
        sum = sum + 1;
    }
}
println(sum);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        // For each i in 0..4, inner loop runs j=0 (sum+1), then j=1 -> continue outer
        // So 4 * 1 = 4 increments
        assert!(r.stdout.contains('4'), "stdout: {}", r.stdout);
    }

    #[test]
    fn unlabeled_break_still_works_inside_labeled_loop() {
        let r = run(
            r#"let count = 0;
outer: for i in 0..3 {
    for j in 0..10 {
        if j == 2 { break; }
        count = count + 1;
    }
}
println(count);"#,
        );
        // 3 outer iters * 2 inner iters each = 6
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('6'), "stdout: {}", r.stdout);
    }

    #[test]
    fn labeled_break_for_with_range() {
        let r = run(
            r#"let x = 0;
outer: for i in 0..10 {
    inner: for j in 0..10 {
        if j == 5 { break inner; }
        x = x + 1;
    }
    if i == 2 { break outer; }
}
println(x);"#,
        );
        // 3 outer iters (0,1,2), each with 5 inner iters = 15
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("15"), "stdout: {}", r.stdout);
    }

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
            stmts: vec![Node::Break {
                span: Span::default(),
            }],
            span: Span::default(),
        };
        let mid = Node::ForInStatement {
            name: "c".into(),
            iterable: Box::new(Node::Identifier {
                name: "xs".into(),
                span: Span::default(),
            }),
            body: Box::new(inner_break),
            invariants: vec![],
            span: Span::default(),
            label: None,
        };
        let outer2 = Node::ForInStatement {
            name: "b".into(),
            iterable: Box::new(Node::Identifier {
                name: "xs".into(),
                span: Span::default(),
            }),
            body: Box::new(mid),
            invariants: vec![],
            span: Span::default(),
            label: None,
        };
        let outer1 = Node::ForInStatement {
            name: "a".into(),
            iterable: Box::new(Node::Identifier {
                name: "xs".into(),
                span: Span::default(),
            }),
            body: Box::new(outer2),
            invariants: vec![],
            span: Span::default(),
            label: None,
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
