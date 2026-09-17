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
use std::collections::HashMap;

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

fn const_expression_key(node: &Node) -> Option<String> {
    match node {
        Node::IntegerLiteral { value, .. } => Some(format!("int:{value}")),
        Node::FloatLiteral { value, .. } => Some(format!("float:{}", value.to_bits())),
        Node::BooleanLiteral { value, .. } => Some(format!("bool:{value}")),
        Node::StringLiteral { value, .. } => Some(format!("string:{value:?}")),
        Node::StringInternLiteral { content, .. } => Some(format!("string:{content:?}")),
        Node::Identifier { name, .. } => Some(format!("identifier:{name}")),
        Node::PrefixExpression {
            operator, right, ..
        } => Some(format!(
            "prefix:{operator}:{}",
            const_expression_key(right)?
        )),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => Some(format!(
            "infix:{}:{operator}:{}",
            const_expression_key(left)?,
            const_expression_key(right)?
        )),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => Some(format!(
            "if:{}:{}:{}",
            const_expression_key(condition)?,
            const_expression_key(consequence)?,
            alternative
                .as_deref()
                .and_then(const_expression_key)
                .unwrap_or_default()
        )),
        Node::ExpressionStatement { expr, .. } => const_expression_key(expr),
        Node::Block { stmts, .. } if stmts.len() == 1 => const_expression_key(&stmts[0]),
        Node::TupleLiteral { items, .. } => {
            let keys = items
                .iter()
                .map(const_expression_key)
                .collect::<Option<Vec<_>>>()?;
            Some(format!("tuple:{keys:?}"))
        }
        _ => None,
    }
}

fn const_declarations_match(
    previous_value: &Node,
    previous_type: Option<&str>,
    current_value: &Node,
    current_type: Option<&str>,
) -> bool {
    previous_type == current_type
        && const_expression_key(previous_value).is_some()
        && const_expression_key(previous_value) == const_expression_key(current_value)
}

fn check_duplicate_top_level_consts(program: &Node, source_path: &str) -> Result<(), String> {
    let Node::Program(statements) = program else {
        return Ok(());
    };

    let mut seen: HashMap<&str, (Span, &Node, Option<&str>)> = HashMap::new();
    for statement in statements {
        let Node::Const {
            name,
            value,
            type_annot,
            span,
        } = &statement.node
        else {
            continue;
        };

        if let Some((previous_span, previous_value, previous_type)) = seen.get(name.as_str()) {
            let kind = if const_declarations_match(
                previous_value,
                *previous_type,
                value,
                type_annot.as_deref(),
            ) {
                "duplicate"
            } else {
                "conflicting"
            };
            let previous_location = format!(
                "{}:{}:{}",
                source_path, previous_span.start.line, previous_span.start.column
            );
            let current_location =
                format!("{}:{}:{}", source_path, span.start.line, span.start.column);
            return Err(diagnostic(
                source_path,
                *span,
                &format!(
                    "{kind} const declaration `{name}`; first declared at {previous_location}, second declared at {current_location}"
                ),
            ));
        }

        seen.insert(
            name.as_str(),
            (*span, value.as_ref(), type_annot.as_deref()),
        );
    }
    Ok(())
}

fn check_nested_consts(program: &Node, source_path: &str) -> Result<(), String> {
    let Node::Program(statements) = program else {
        return Ok(());
    };

    for statement in statements {
        let mut error = None;
        crate::uniqueness_walk::walk_children(&statement.node, &mut |node| {
            if error.is_some() {
                return;
            }
            if let Node::Const { span, .. } = node {
                error = Some(diagnostic(
                    source_path,
                    *span,
                    "function-scoped `const` declarations are not supported; use `let` instead",
                ));
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
    }
    Ok(())
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
        }
    });

    if let Some(err) = error {
        return Err(err);
    }
    check_duplicate_top_level_consts(program, source_path)?;
    check_nested_consts(program, source_path)
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

    // ── Extended malformed-input regression corpus (RES-3770) ────────────────

    #[test]
    fn check_malformed_undefined_const_reference() {
        let src = "const X = UNDEFINED_CONST;";
        let err = run_expect_err(src);
        assert!(
            err.contains("not a compile-time constant")
                || err.contains("undefined")
                || err.contains("not found"),
            "expected compile-time constant error, got: {err:?}"
        );
    }

    #[test]
    fn check_malformed_mixed_type_concat() {
        let src = "const RESULT = \"string\" + 42;";
        let err = run_expect_err(src);
        assert!(
            err.contains("not supported") || err.contains("type") || err.contains("incompatible"),
            "expected unsupported operator error, got: {err:?}"
        );
    }

    #[test]
    fn check_malformed_variable_reference_in_const() {
        let src = "let var = 5;\nconst X = var + 1;";
        let err = run_expect_err(src);
        assert!(
            err.contains("not a compile-time constant")
                || err.contains("not constant")
                || err.contains("invalid"),
            "expected non-constant error, got: {err:?}"
        );
    }

    #[test]
    fn check_malformed_circular_ref_indirect() {
        let src = "const X = Y;\nconst Y = X;";
        let err = run_expect_err(src);
        assert!(
            err.contains("not a compile-time constant")
                || err.contains("circular")
                || err.contains("undefined"),
            "expected circular/undefined error, got: {err:?}"
        );
    }

    #[test]
    fn check_valid_const_with_comparison() {
        // Valid: comparison is allowed in const expressions
        let out = run("const CMP = 5 > 3;\nprintln(to_string(CMP));");
        assert!(out.contains("true"), "got: {out:?}");
    }

    #[test]
    fn check_valid_const_with_negation() {
        // Valid case: unary operations work in const expressions
        let out = run("const NEG = -5;\nprintln(to_string(NEG));");
        assert!(out.contains("-5"), "got: {out:?}");
    }

    #[test]
    fn check_malformed_const_division_by_zero() {
        let src = "const X = 10;\nconst DIV = X / 0;";
        let err = run_expect_err(src);
        assert!(
            err.contains("division") || err.contains("zero"),
            "expected division error, got: {err:?}"
        );
    }

    #[test]
    fn check_malformed_const_with_side_effects() {
        // println in const initializer should fail (parser error on map syntax)
        let src = "const X = { println(1); 5 };";
        let err = run_expect_err(src);
        assert!(
            !err.is_empty(),
            "expected error on const with side effects, got: {err:?}"
        );
    }
}
