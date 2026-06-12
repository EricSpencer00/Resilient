//! RES-2580: extended compile-time constant evaluation.
//!
//! Extends `Interpreter::eval_const_expr`
//! previously rejected "not valid constant expression":
//!
//! - **String concatenation**: `const GREETING = "Hello, " + NAME;`
//! - **String ordering**: `const OK = "alpha" < "beta";`
//! - **Bitwise operators**: `const MASK = FLAGS & 0xFF;`, `|`, `^`, `<<`, `>>`
//! - **Conditional expressions**: `const MAX = if A > B { A } else { B };`
//! - **Single-expression blocks**: `const X = { 1 + 2 };`
//! - **Tuple literals**: `const PAIR = (1, 2);`
//!
//! All new cases live in `Interpreter::eval_const_expr` in `lib.rs`.
//! This module now also validates malformed const declarations so
//! recovery placeholders do not leak into later phases.

use crate::Node;
use crate::span::Span;

fn diagnostic(source_path: &str, span: Span, message: &str) -> String {
    format!(
        "{}:{}:{}: error: {}",
        source_path, span.start.line, span.start.column, message
    )
}

fn is_missing_initializer(node: &Node) -> bool {
    matches!(
        node,
        Node::IntegerLiteral {
            value: 0,
            span,
        } if *span == Span::default()
    )
}

fn validate_const_expr(node: &Node, source_path: &str) -> Result<(), String> {
    match node {
        Node::Identifier { .. }
        | Node::IntegerLiteral { .. }
        | Node::FloatLiteral { .. }
        | Node::StringLiteral { .. }
        | Node::StringInternLiteral { .. }
        | Node::BytesLiteral { .. }
        | Node::CharLiteral { .. }
        | Node::BooleanLiteral { .. } => Ok(()),
        Node::ExpressionStatement { expr, .. } => validate_const_expr(expr, source_path),
        Node::PrefixExpression { right, .. } => validate_const_expr(right, source_path),
        Node::InfixExpression { left, right, .. } => {
            validate_const_expr(left, source_path)?;
            validate_const_expr(right, source_path)
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            validate_const_expr(condition, source_path)?;
            validate_const_expr(consequence, source_path)?;
            if let Some(alt) = alternative {
                validate_const_expr(alt, source_path)?;
            }
            Ok(())
        }
        Node::Block { stmts, span } => {
            if stmts.len() != 1 {
                return Err(diagnostic(
                    source_path,
                    *span,
                    "invalid const expression: blocks in const initializers must contain exactly one expression",
                ));
            }

            let inner = match &stmts[0] {
                Node::ExpressionStatement { expr, .. } => expr.as_ref(),
                other => other,
            };
            validate_const_expr(inner, source_path)
        }
        Node::TupleLiteral { items, .. } => items
            .iter()
            .try_for_each(|item| validate_const_expr(item, source_path)),
        Node::CallExpression { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: function calls are not allowed",
        )),
        Node::ArrayLiteral { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: array literals are not allowed",
        )),
        Node::StructLiteral { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: struct literals are not allowed",
        )),
        Node::FieldAccess { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: field access is not allowed",
        )),
        Node::IndexExpression { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: index expressions are not allowed",
        )),
        Node::Match { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: match expressions are not allowed",
        )),
        Node::LetStatement { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: let statements are not allowed",
        )),
        Node::ReturnStatement { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: return statements are not allowed",
        )),
        Node::WhileStatement { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: while statements are not allowed",
        )),
        Node::ForInStatement { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: for-in statements are not allowed",
        )),
        Node::Break { span } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: break is not allowed",
        )),
        Node::Continue { span } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: continue is not allowed",
        )),
        Node::BreakWith { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: break values are not allowed",
        )),
        Node::BreakLabel { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: labeled break is not allowed",
        )),
        Node::ContinueLabel { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: labeled continue is not allowed",
        )),
        Node::FieldAssignment { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: field assignment is not allowed",
        )),
        Node::IndexAssignment { span, .. } => Err(diagnostic(
            source_path,
            *span,
            "invalid const expression: index assignment is not allowed",
        )),
        other => Err(diagnostic(
            source_path,
            span_of(other),
            "invalid const expression: this expression form is not allowed",
        )),
    }
}

fn span_of(node: &Node) -> Span {
    match node {
        Node::Identifier { span, .. }
        | Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::StringInternLiteral { span, .. }
        | Node::BytesLiteral { span, .. }
        | Node::CharLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::CallExpression { span, .. }
        | Node::Match { span, .. }
        | Node::StructLiteral { span, .. }
        | Node::FieldAccess { span, .. }
        | Node::FieldAssignment { span, .. }
        | Node::ArrayLiteral { span, .. }
        | Node::IndexExpression { span, .. }
        | Node::IndexAssignment { span, .. }
        | Node::ExpressionStatement { span, .. }
        | Node::IfStatement { span, .. }
        | Node::Block { span, .. }
        | Node::TupleLiteral { span, .. }
        | Node::LetStatement { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. }
        | Node::Break { span }
        | Node::Continue { span }
        | Node::BreakWith { span, .. }
        | Node::BreakLabel { span, .. }
        | Node::ContinueLabel { span, .. } => *span,
        _ => Span::default(),
    }
}

/// Validate const declarations before const evaluation runs.
///
/// The parser can recover from malformed `const` statements by
/// synthesizing placeholder nodes. Reject those here so later phases
/// never see a structurally-invalid declaration.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let mut error = None;

    crate::uniqueness_walk::visit(program, &mut |node| {
        if error.is_some() {
            return;
        }

        let Node::Const {
            name,
            value,
            type_annot,
            span,
        } = node
        else {
            return;
        };

        if name.trim().is_empty() {
            error = Some(diagnostic(
                source_path,
                *span,
                "invalid const declaration: missing name",
            ));
            return;
        }

        if is_missing_initializer(value) {
            let message = if type_annot.is_some() {
                "invalid const declaration: type annotations require an initializer"
            } else {
                "invalid const declaration: missing initializer"
            };
            error = Some(diagnostic(source_path, *span, message));
            return;
        }

        if let Err(err) = validate_const_expr(value, source_path) {
            error = Some(err);
        }
    });

    match error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::check;
    use crate::Node;
    use crate::run_program;
    use crate::span::{Pos, Span, Spanned};

    fn run(src: &str) -> String {
        let r = run_program(src);
        assert!(r.ok, "program failed: {:?}", r.errors);
        r.stdout
    }

    fn run_expect_err(src: &str) -> String {
        let r = run_program(src);
        assert!(!r.ok, "expected error but program succeeded");
        r.errors.join("\n")
    }

    fn pos(line: usize, column: usize) -> Pos {
        Pos::new(line, column, 0)
    }

    fn span(line: usize, column: usize) -> Span {
        Span::new(pos(line, column), pos(line, column + 1))
    }

    fn spanned(node: Node, line: usize, column: usize) -> Spanned<Node> {
        Spanned {
            node,
            span: span(line, column),
        }
    }

    fn const_stmt(
        name: &str,
        value: Node,
        type_annot: Option<&str>,
        line: usize,
        column: usize,
    ) -> Spanned<Node> {
        spanned(
            Node::Const {
                name: name.to_string(),
                value: Box::new(value),
                type_annot: type_annot.map(str::to_string),
                span: span(line, column),
            },
            line,
            column,
        )
    }

    fn program(stmt: Spanned<Node>) -> Node {
        Node::Program(vec![stmt])
    }

    #[test]
    fn const_string_concat() {
        let out = run(r#"
const FIRST = "Hello";
const REST = ", world";
const FULL = FIRST + REST;
println(FULL);
"#);
        assert!(out.contains("Hello, world"), "got: {out:?}");
    }

    #[test]
    fn const_string_ordering() {
        let out = run(r#"
const A = "alpha";
const B = "beta";
const ORDERED = A < B;
println(to_string(ORDERED));
"#);
        assert!(out.contains("true"), "got: {out:?}");
    }

    #[test]
    fn const_bitwise_and() {
        let out = run(r#"
const FLAGS = 0xFF;
const MASK = 0x0F;
const LOWER = FLAGS & MASK;
println(to_string(LOWER));
"#);
        assert!(out.contains("15"), "got: {out:?}");
    }

    #[test]
    fn const_bitwise_or() {
        let out = run(r#"
const A = 0b1010;
const B = 0b0101;
const C = A | B;
println(to_string(C));
"#);
        assert!(out.contains("15"), "got: {out:?}");
    }

    #[test]
    fn const_bitwise_xor() {
        let out = run(r#"
const A = 0xFF;
const B = 0xF0;
const C = A ^ B;
println(to_string(C));
"#);
        assert!(out.contains("15"), "got: {out:?}");
    }

    #[test]
    fn const_shift() {
        let out = run(r#"
const BASE = 1;
const SHIFTED = BASE << 4;
println(to_string(SHIFTED));
"#);
        assert!(out.contains("16"), "got: {out:?}");
    }

    #[test]
    fn const_conditional_true_branch() {
        let out = run(r#"
const A = 10;
const B = 5;
const MAX = if A > B { A } else { B };
println(to_string(MAX));
"#);
        assert!(out.contains("10"), "got: {out:?}");
    }

    #[test]
    fn const_conditional_false_branch() {
        let out = run(r#"
const A = 3;
const B = 7;
const MAX = if A > B { A } else { B };
println(to_string(MAX));
"#);
        assert!(out.contains("7"), "got: {out:?}");
    }

    #[test]
    fn const_tuple() {
        let out = run(r#"
const PAIR = (1, 2);
let (a, b) = PAIR;
println(to_string(a + b));
"#);
        assert!(out.contains("3"), "got: {out:?}");
    }

    #[test]
    fn const_circular_reference_errors() {
        let err = run_expect_err("const X = X;");
        assert!(
            err.contains("circular"),
            "expected circular error, got: {err:?}"
        );
    }

    #[test]
    fn const_decl_missing_name_is_rejected() {
        let program = program(const_stmt(
            "",
            Node::IntegerLiteral {
                value: 1,
                span: span(1, 14),
            },
            None,
            1,
            1,
        ));

        let err = check(&program, "test.rz").unwrap_err();
        assert_eq!(
            err,
            "test.rz:1:1: error: invalid const declaration: missing name"
        );
    }

    #[test]
    fn const_decl_missing_initializer_is_rejected() {
        let program = program(const_stmt(
            "ANSWER",
            Node::IntegerLiteral {
                value: 0,
                span: Span::default(),
            },
            None,
            3,
            5,
        ));

        let err = check(&program, "test.rz").unwrap_err();
        assert_eq!(
            err,
            "test.rz:3:5: error: invalid const declaration: missing initializer"
        );
    }

    #[test]
    fn annotated_const_decl_without_initializer_is_rejected() {
        let program = program(const_stmt(
            "VALUE",
            Node::IntegerLiteral {
                value: 0,
                span: Span::default(),
            },
            Some("int"),
            7,
            2,
        ));

        let err = check(&program, "test.rz").unwrap_err();
        assert_eq!(
            err,
            "test.rz:7:2: error: invalid const declaration: type annotations require an initializer"
        );
    }
}
