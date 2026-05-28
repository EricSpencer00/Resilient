//! RES-2660: `static_assert(expr, msg)` — compile-time assertion.
//!
//! Evaluates the condition expression using the const evaluator at
//! compile time. If the condition is `false`, the compiler emits a
//! hard error with the user-supplied message and source location.
//! If `true`, the assertion is silently elided — zero runtime cost,
//! zero bytecode emitted.
//!
//! This is essential for safety-critical embedded systems where
//! compile-time validation of buffer sizes, alignment constraints,
//! and configuration bounds prevents classes of bugs that would be
//! catastrophic at runtime.
//!
//! ## Syntax
//!
//! ```text
//! static_assert(<const-expr>, "message");
//! ```
//!
//! Both arguments are required. The condition must be a
//! const-evaluable boolean expression (same rules as `const`
//! declarations: literals, arithmetic, comparisons, and references
//! to previously declared constants). The message must be a string
//! literal.
//!
//! ## Feature isolation
//!
//! All logic lives here. Core files (`lib.rs`, `typechecker.rs`,
//! `lexer_logos.rs`) have only the minimal extension-point entries:
//! one `Token::StaticAssert` variant, one keyword mapping, one
//! `Node::StaticAssert` variant, one `parse_statement` dispatch arm,
//! and one `<EXTENSION_PASSES>` call.

use crate::span::Span;
use crate::{Node, Parser, Token, Value};
use std::collections::HashMap;

/// Parse a `static_assert(expr, msg);` statement.
///
/// Called from `Parser::parse_statement` when the current token is
/// `Token::StaticAssert`. Consumes the entire statement including
/// the trailing semicolon (which `parse_program` also skips, but
/// the double-skip on `;` is harmless — it's the same pattern as
/// `parse_assert`).
pub(crate) fn parse(parser: &mut Parser) -> Node {
    let start_span = parser.span_at_current();
    parser.next_token(); // skip `static_assert`

    if parser.current_token != Token::LeftParen {
        let tok = parser.current_token.clone();
        parser.record_error(format!("Expected '(' after 'static_assert', found {}", tok));
        return Node::StaticAssert {
            condition: Box::new(Node::BooleanLiteral {
                value: true,
                span: Span::default(),
            }),
            message: String::new(),
            span: start_span,
        };
    }
    parser.next_token(); // skip '('

    let condition = parser.parse_expression(0).unwrap_or(Node::BooleanLiteral {
        value: true,
        span: Span::default(),
    });
    parser.next_token(); // advance past last token of condition

    if parser.current_token != Token::Comma {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "static_assert requires two arguments: condition and message. \
             Expected ',' after condition, found {}",
            tok
        ));
        return Node::StaticAssert {
            condition: Box::new(condition),
            message: String::new(),
            span: start_span,
        };
    }
    parser.next_token(); // skip ','

    let message = match &parser.current_token {
        Token::StringLiteral(s) => s.clone(),
        other => {
            parser.record_error(format!(
                "static_assert message must be a string literal, found {}",
                other
            ));
            String::new()
        }
    };
    parser.next_token(); // skip message

    if parser.current_token != Token::RightParen {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "Expected ')' after static_assert message, found {}",
            tok
        ));
    }

    Node::StaticAssert {
        condition: Box::new(condition),
        message,
        span: start_span,
    }
}

/// Evaluate all `static_assert` statements in the program.
///
/// Called from the `<EXTENSION_PASSES>` block in `typechecker.rs`.
/// Resolves all `const` declarations first (same logic as
/// `Interpreter::const_eval_program`), then evaluates each
/// `static_assert` condition. A `false` result is a hard compile
/// error; `true` is silently accepted.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let statements = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    // First pass: resolve all top-level `const` declarations so
    // static_assert conditions can reference them. This mirrors
    // `Interpreter::const_eval_program` but is self-contained —
    // the typechecker pass doesn't have access to the interpreter's
    // const table.
    let mut consts: HashMap<String, Value> = HashMap::new();
    for stmt in statements {
        if let Node::Const { name, value, .. } = &stmt.node {
            let mut evaluating: Vec<String> = vec![name.clone()];
            match crate::Interpreter::eval_const_expr(value, &consts, &mut evaluating) {
                Ok(v) => {
                    consts.insert(name.clone(), v);
                }
                Err(_) => {
                    // Const eval failure is reported elsewhere; skip.
                }
            }
        }
    }

    // Second pass: evaluate each static_assert.
    let mut errors: Vec<String> = Vec::new();

    for stmt in statements {
        let Node::StaticAssert {
            condition,
            message,
            span,
        } = &stmt.node
        else {
            continue;
        };

        let mut evaluating: Vec<String> = Vec::new();
        match eval_const_bool(condition, &consts, &mut evaluating) {
            Ok(true) => {
                // Assertion passed — nothing to do.
            }
            Ok(false) => {
                errors.push(format!(
                    "{}:{}:{}: error: static assertion failed: {}",
                    source_path, span.start.line, span.start.column, message
                ));
            }
            Err(e) => {
                errors.push(format!(
                    "{}:{}:{}: error: static_assert condition is not a \
                     compile-time constant expression: {}",
                    source_path, span.start.line, span.start.column, e
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// Evaluate static_assert statements using pre-resolved constants.
///
/// Called from `Interpreter::eval_program` after `const_eval_program`
/// has populated the const table. This is the interpreter-path entry
/// point; the typechecker path uses `check()` which resolves consts
/// itself.
pub(crate) fn check_with_consts(
    statements: &[crate::span::Spanned<Node>],
    consts: &std::rc::Rc<HashMap<String, Value>>,
) -> Result<(), String> {
    let mut errors: Vec<String> = Vec::new();

    for stmt in statements {
        let Node::StaticAssert {
            condition,
            message,
            span,
        } = &stmt.node
        else {
            continue;
        };

        let mut evaluating: Vec<String> = Vec::new();
        match eval_const_bool(condition, consts, &mut evaluating) {
            Ok(true) => {}
            Ok(false) => {
                errors.push(format!(
                    "static assertion failed: {} (at line {})",
                    message, span.start.line
                ));
            }
            Err(e) => {
                errors.push(format!(
                    "static_assert condition is not a compile-time constant \
                     expression: {} (at line {})",
                    e, span.start.line
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// Evaluate a const expression expected to produce a boolean.
///
/// Delegates to `Interpreter::eval_const_expr` for the heavy lifting,
/// then checks that the result is actually `Value::Bool`.
fn eval_const_bool(
    node: &Node,
    consts: &HashMap<String, Value>,
    evaluating: &mut Vec<String>,
) -> Result<bool, String> {
    let value = crate::Interpreter::eval_const_expr(node, consts, evaluating)?;
    match value {
        Value::Bool(b) => Ok(b),
        other => Err(format!("expected boolean, got {}", other)),
    }
}

#[cfg(test)]
mod tests {
    use crate::parse;

    #[test]
    fn parse_static_assert_basic() {
        let src = "static_assert(1 == 1, \"must hold\");";
        let (program, errors) = parse(src);
        assert!(errors.is_empty(), "unexpected parse errors: {:?}", errors);
        match &program {
            crate::Node::Program(stmts) => {
                assert_eq!(stmts.len(), 1);
                match &stmts[0].node {
                    crate::Node::StaticAssert { message, .. } => {
                        assert_eq!(message, "must hold");
                    }
                    other => panic!("expected StaticAssert, got {:?}", other),
                }
            }
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn parse_static_assert_with_const() {
        let src = r#"
            const SIZE: int = 1024;
            static_assert(SIZE <= 4096, "buffer too large");
        "#;
        let (program, errors) = parse(src);
        assert!(errors.is_empty(), "unexpected parse errors: {:?}", errors);
        match &program {
            crate::Node::Program(stmts) => {
                // const + static_assert
                assert_eq!(stmts.len(), 2);
                assert!(matches!(&stmts[1].node, crate::Node::StaticAssert { .. }));
            }
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn static_assert_pass_evaluates() {
        let src = r#"
            const SIZE: int = 1024;
            static_assert(SIZE <= 4096, "buffer too large for SRAM");
            static_assert(SIZE % 4 == 0, "buffer must be 4-byte aligned");
        "#;
        let result = crate::run_program(src);
        assert!(
            result.errors.is_empty(),
            "static_assert should pass: {:?}",
            result.errors
        );
    }

    #[test]
    fn static_assert_fail_produces_error() {
        let src = r#"
            const SIZE: int = 8192;
            static_assert(SIZE <= 4096, "buffer too large for SRAM");
        "#;
        let result = crate::run_program(src);
        assert!(!result.errors.is_empty(), "static_assert should fail");
        let err = result.errors.join("\n");
        assert!(
            err.contains("static assertion failed"),
            "error should mention 'static assertion failed', got: {}",
            err
        );
        assert!(
            err.contains("buffer too large for SRAM"),
            "error should include user message, got: {}",
            err
        );
    }

    #[test]
    fn static_assert_non_bool_errors() {
        let src = r#"
            static_assert(42, "not a bool");
        "#;
        let result = crate::run_program(src);
        assert!(!result.errors.is_empty(), "non-bool condition should error");
        let err = result.errors.join("\n");
        assert!(
            err.contains("expected boolean"),
            "should say 'expected boolean', got: {}",
            err
        );
    }

    #[test]
    fn static_assert_with_arithmetic() {
        let src = r#"
            const A: int = 10;
            const B: int = 20;
            static_assert(A + B == 30, "arithmetic check");
            static_assert(A * B == 200, "multiplication check");
        "#;
        let result = crate::run_program(src);
        assert!(
            result.errors.is_empty(),
            "arithmetic static_assert should pass: {:?}",
            result.errors
        );
    }

    #[test]
    fn static_assert_multiple_failures() {
        let src = r#"
            const X: int = 100;
            static_assert(X < 50, "X too large");
            static_assert(X % 3 == 0, "X not divisible by 3");
        "#;
        let result = crate::run_program(src);
        let err = result.errors.join("\n");
        assert!(
            err.contains("X too large"),
            "should report first failure: {}",
            err
        );
        assert!(
            err.contains("X not divisible by 3"),
            "should report second failure: {}",
            err
        );
    }

    #[test]
    fn static_assert_no_runtime_cost() {
        // static_assert should be evaluated at compile time only;
        // the interpreter should not see it as a runtime statement.
        let src = r#"
            const N: int = 5;
            static_assert(N > 0, "N must be positive");
            fn main(int _d) { return N; }
            main(0);
        "#;
        let result = crate::run_program(src);
        assert!(
            result.errors.is_empty(),
            "should compile and run cleanly: {:?}",
            result.errors
        );
    }
}
