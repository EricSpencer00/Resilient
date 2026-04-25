//! RES-222: Loop invariants — `invariant EXPR;` inside `while` / `for`
//! bodies. The invariant is checked at runtime at the top of every
//! iteration; a violation halts execution with
//! `runtime error: loop invariant violated at L:C`.
//!
//! Two surface forms feed into the same machinery:
//!
//! 1. **Pre-body** (RES-132a, already in the parser): `while c
//!    invariant p { ... }`. Stored on `Node::WhileStatement.invariants`.
//! 2. **Statement-form** (this ticket): `while c { invariant p; ... }`.
//!    Stored as a `Node::InvariantStatement` at the top of the body
//!    `Block`. The interpreter sweeps these out at iteration top so
//!    they aren't double-evaluated when the block runs normally.
//!
//! `invariant` outside any loop body is a hard typecheck error —
//! the standalone form has no useful runtime semantics.
//!
//! Z3 inductive verification is left to a follow-up (RES-318); this
//! ticket lands the parser, typechecker, and runtime check only.

use crate::Node;
use crate::span::Span;

/// Parse `invariant EXPR;` as a statement. Cursor enters on
/// `Token::Invariant`; on return cursor sits on the last token of
/// the expression (mirroring the `assert` / `assume` parsers — the
/// outer `parse_block_statement` consumes the trailing `;`).
pub(crate) fn parse_invariant_statement(parser: &mut crate::Parser) -> Node {
    let stmt_span = parser.span_at_current();
    parser.next_token(); // skip `invariant`

    let parenthesized = parser.current_token == crate::Token::LeftParen;
    if parenthesized {
        parser.next_token(); // skip `(`
    }

    let expr = parser.parse_expression(0).unwrap_or(Node::BooleanLiteral {
        value: true,
        span: Span::default(),
    });
    parser.next_token(); // RES-014: move past last token of expression

    if parenthesized && parser.current_token != crate::Token::RightParen {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "Expected ')' after invariant expression, found {}",
            tok
        ));
        // leave cursor on `)`; outer loop advances to `;`.
    }

    Node::InvariantStatement {
        expr: Box::new(expr),
        span: stmt_span,
    }
}

/// Program-level pass: every `Node::InvariantStatement` must sit
/// inside a `while` / `for` body. Anywhere else is a hard error.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let Node::Program(statements) = program else {
        return Ok(());
    };
    for spanned in statements {
        walk(&spanned.node, /*in_loop=*/ false, source_path)?;
    }
    Ok(())
}

fn walk(node: &Node, in_loop: bool, source_path: &str) -> Result<(), String> {
    match node {
        Node::InvariantStatement { span, .. } => {
            if !in_loop {
                return Err(format!(
                    "{}:{}:{}: `invariant` is only valid inside a `while` or `for` loop body",
                    source_path, span.start.line, span.start.column
                ));
            }
            Ok(())
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            walk(body, /*in_loop=*/ true, source_path)
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk(s, in_loop, source_path)?;
            }
            Ok(())
        }
        Node::Function { body, .. } => walk(body, /*in_loop=*/ false, source_path),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            walk(consequence, in_loop, source_path)?;
            if let Some(alt) = alternative {
                walk(alt, in_loop, source_path)?;
            }
            Ok(())
        }
        Node::LiveBlock { body, .. } => walk(body, in_loop, source_path),
        Node::TryCatch { body, handlers, .. } => {
            for s in body {
                walk(s, in_loop, source_path)?;
            }
            for (_v, h) in handlers {
                for s in h {
                    walk(s, in_loop, source_path)?;
                }
            }
            Ok(())
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                walk(m, in_loop, source_path)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Typecheck an `invariant` body — must be Bool. Called from the
/// `check_node` arm in `typechecker.rs`.
pub(crate) fn typecheck_invariant_statement(
    tc: &mut crate::typechecker::TypeChecker,
    expr: &Node,
) -> Result<crate::typechecker::Type, String> {
    let t = tc.check_node(expr)?;
    if t != crate::typechecker::Type::Bool && t != crate::typechecker::Type::Any {
        return Err(format!("Loop invariant must be a boolean, got {}", t));
    }
    Ok(crate::typechecker::Type::Void)
}

/// Collect the top-level `InvariantStatement` expressions inside a
/// loop body. Nested invariants (inside an `if` etc.) are ignored —
/// they only fire if the typechecker walked through, but they don't
/// participate in the iteration-top check. Loop-top invariants must
/// hold over the loop's full state, not a branch-conditional state.
pub(crate) fn collect_body_invariants(body: &Node) -> Vec<&Node> {
    let mut out = Vec::new();
    if let Node::Block { stmts, .. } = body {
        for s in stmts {
            if let Node::InvariantStatement { expr, .. } = s {
                out.push(expr.as_ref());
            }
        }
    }
    out
}

/// Runtime check called by the interpreter at the top of every loop
/// iteration. Evaluates each invariant; the first false one halts.
///
/// `field_invariants` is the `invariants: Vec<Node>` slot on
/// `WhileStatement` / `ForInStatement` (RES-132a pre-body form).
/// `body_invariants` is the slice of expressions extracted from the
/// body block by [`collect_body_invariants`] (RES-222 statement form).
pub(crate) fn check_invariants_at_iteration(
    interp: &mut crate::Interpreter,
    field_invariants: &[Node],
    body_invariants: &[&Node],
    loop_span: &Span,
) -> Result<(), String> {
    for inv in field_invariants {
        evaluate_one(interp, inv, loop_span)?;
    }
    for inv in body_invariants {
        evaluate_one(interp, inv, loop_span)?;
    }
    Ok(())
}

fn evaluate_one(
    interp: &mut crate::Interpreter,
    inv: &Node,
    loop_span: &Span,
) -> Result<(), String> {
    let v = interp.eval(inv)?;
    if !interp.is_truthy(&v) {
        let inv_span = node_span(inv).unwrap_or(loop_span);
        return Err(format!(
            "runtime error: loop invariant violated at {}:{}",
            inv_span.start.line, inv_span.start.column
        ));
    }
    Ok(())
}

fn node_span(node: &Node) -> Option<&Span> {
    match node {
        Node::InfixExpression { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::Identifier { span, .. }
        | Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::CallExpression { span, .. } => Some(span),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Interpreter, Lexer, Parser};

    fn parse(src: &str) -> Node {
        let lexer = Lexer::new(src.to_string());
        let mut parser = Parser::new(lexer);
        parser.parse_program()
    }

    /// Parse and run a program. Typechecking is intentionally skipped:
    /// the for-in typechecker does not currently bind the loop variable
    /// (a pre-existing limitation orthogonal to RES-222), so for-in
    /// invariants that reference the loop var would spuriously fail
    /// the check pass. We do still apply the loop_invariants check
    /// itself — that's the part being tested.
    fn run(src: &str) -> Result<crate::Value, String> {
        let p = parse(src);
        check(&p, "<test>").map_err(|e| format!("loop_invariants check: {}", e))?;
        let mut interp = Interpreter::new();
        interp.eval(&p)
    }

    #[test]
    fn invariant_inside_while_body_parses() {
        let src = r#"
            fn main() {
                let mut i = 0;
                while i < 10 {
                    invariant i >= 0;
                    i = i + 1;
                }
            }
            main();
        "#;
        let p = parse(src);
        // Body invariants should have been collected at parse time
        // (as plain InvariantStatement nodes inside the Block).
        let mut found = 0usize;
        walk_count(&p, &mut found);
        assert!(found >= 1, "expected at least one InvariantStatement");
    }

    fn walk_count(n: &Node, out: &mut usize) {
        match n {
            Node::InvariantStatement { .. } => *out += 1,
            Node::Program(stmts) => {
                for s in stmts {
                    walk_count(&s.node, out);
                }
            }
            Node::Block { stmts, .. } => {
                for s in stmts {
                    walk_count(s, out);
                }
            }
            Node::Function { body, .. }
            | Node::WhileStatement { body, .. }
            | Node::ForInStatement { body, .. }
            | Node::LiveBlock { body, .. } => walk_count(body, out),
            Node::IfStatement {
                consequence,
                alternative,
                ..
            } => {
                walk_count(consequence, out);
                if let Some(alt) = alternative {
                    walk_count(alt, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn invariant_outside_loop_is_error() {
        let src = r#"
            fn main() {
                invariant 1 > 0;
            }
            main();
        "#;
        let p = parse(src);
        let res = check(&p, "<test>");
        assert!(
            res.is_err(),
            "expected hard error for invariant outside loop"
        );
        let msg = res.unwrap_err();
        assert!(msg.contains("only valid inside"), "got: {}", msg);
    }

    #[test]
    fn invariant_inside_loop_body_passes_check() {
        let src = r#"
            fn main() {
                let mut i = 0;
                while i < 3 {
                    invariant i >= 0;
                    i = i + 1;
                }
            }
            main();
        "#;
        let p = parse(src);
        assert!(check(&p, "<test>").is_ok());
    }

    #[test]
    fn invariant_in_for_loop_body_passes_check() {
        let src = r#"
            fn main() {
                for x in [1, 2, 3] {
                    invariant x > 0;
                }
            }
            main();
        "#;
        let p = parse(src);
        assert!(check(&p, "<test>").is_ok());
    }

    #[test]
    fn invariant_in_top_level_block_is_error() {
        let src = r#"
            invariant true;
            fn main() {}
            main();
        "#;
        let p = parse(src);
        assert!(check(&p, "<test>").is_err());
    }

    #[test]
    fn invariant_inside_if_inside_loop_passes() {
        let src = r#"
            fn main() {
                let mut i = 0;
                while i < 3 {
                    if i > 0 {
                        invariant i >= 0;
                    }
                    i = i + 1;
                }
            }
            main();
        "#;
        let p = parse(src);
        // Nested-but-still-inside-loop invariants are allowed; only
        // top-of-body ones participate in the iteration check.
        assert!(check(&p, "<test>").is_ok());
    }

    #[test]
    fn collect_body_invariants_picks_top_level_only() {
        let src = r#"
            fn main() {
                let mut i = 0;
                while i < 3 {
                    invariant i >= 0;
                    invariant i < 100;
                    if i > 0 {
                        invariant i >= 1;
                    }
                    i = i + 1;
                }
            }
            main();
        "#;
        let p = parse(src);
        // Drill into the first WhileStatement and count.
        let body = first_while_body(&p).expect("expected a while loop");
        let invs = collect_body_invariants(body);
        assert_eq!(
            invs.len(),
            2,
            "expected 2 top-level invariants, got {}",
            invs.len()
        );
    }

    #[test]
    fn runtime_invariant_holds_for_correct_loop() {
        let src = r#"
            fn main() {
                let i = 0;
                while i < 5 {
                    invariant i >= 0;
                    invariant i <= 5;
                    i = i + 1;
                }
            }
            main();
        "#;
        let r = run(src);
        assert!(r.is_ok(), "expected ok, got {:?}", r);
    }

    #[test]
    fn runtime_invariant_violation_halts() {
        // Body bumps i past the bound, so on the next iteration top
        // the invariant `i < 1` no longer holds.
        let src = r#"
            fn main() {
                let i = 0;
                while i < 5 {
                    invariant i < 1;
                    i = i + 1;
                }
            }
            main();
        "#;
        let r = run(src);
        let msg = r.expect_err("expected invariant violation");
        assert!(
            msg.contains("loop invariant violated"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn runtime_invariant_holds_for_for_loop() {
        let src = r#"
            fn main() {
                for x in [1, 2, 3] {
                    invariant x > 0;
                }
            }
            main();
        "#;
        let r = run(src);
        assert!(r.is_ok(), "expected ok, got {:?}", r);
    }

    #[test]
    fn runtime_invariant_for_loop_violation_halts() {
        let src = r#"
            fn main() {
                for x in [1, -2, 3] {
                    invariant x > 0;
                }
            }
            main();
        "#;
        let r = run(src);
        let msg = r.expect_err("expected invariant violation");
        assert!(
            msg.contains("loop invariant violated"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn invariant_evaluated_before_first_iteration() {
        // The body never runs because the condition is false on entry,
        // but the invariant is still checked once. With i=10 and the
        // invariant `i < 5`, we should still see the violation.
        let src = r#"
            fn main() {
                let i = 10;
                while i < 5 {
                    invariant i < 5;
                    i = i + 1;
                }
            }
            main();
        "#;
        let r = run(src);
        let msg = r.expect_err("expected invariant violation on entry");
        assert!(
            msg.contains("loop invariant violated"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn pre_body_invariant_form_also_runtime_checked() {
        // RES-132a-syntax (`while c invariant p { ... }`) shares the
        // same iteration-top check.
        let src = r#"
            fn main() {
                let i = 0;
                while i < 5 invariant (i >= 0) {
                    i = i + 1;
                }
            }
            main();
        "#;
        assert!(run(src).is_ok());

        let bad = r#"
            fn main() {
                let i = 10;
                while i < 5 invariant (i < 5) {
                    i = i + 1;
                }
            }
            main();
        "#;
        let r = run(bad);
        let msg = r.expect_err("expected invariant violation");
        assert!(
            msg.contains("loop invariant violated"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn typecheck_rejects_non_bool_invariant() {
        // Use a top-level loop so the typechecker doesn't have to walk
        // through a for-in (which doesn't bind its loop var today).
        let src = r#"
            fn main() {
                let i = 0;
                while i < 5 {
                    invariant 42;
                    i = i + 1;
                }
            }
            main();
        "#;
        let p = parse(src);
        let mut tc = crate::typechecker::TypeChecker::new();
        let res = tc.check_program(&p);
        assert!(
            res.is_err(),
            "expected typecheck error for non-bool invariant"
        );
        let msg = res.unwrap_err();
        assert!(msg.to_lowercase().contains("invariant"), "got: {}", msg);
    }

    fn first_while_body(n: &Node) -> Option<&Node> {
        match n {
            Node::WhileStatement { body, .. } => Some(body),
            Node::Program(stmts) => {
                for s in stmts {
                    if let Some(b) = first_while_body(&s.node) {
                        return Some(b);
                    }
                }
                None
            }
            Node::Function { body, .. } => first_while_body(body),
            Node::Block { stmts, .. } => {
                for s in stmts {
                    if let Some(b) = first_while_body(s) {
                        return Some(b);
                    }
                }
                None
            }
            _ => None,
        }
    }
}
