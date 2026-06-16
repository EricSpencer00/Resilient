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

    // ── Malformed-input regression corpus (RES-3214) ──────────────
    #[test]
    fn corpus_const_empty_program_valid() {
        let prog = Node::Program(vec![]);
        assert!(check(&prog, "test.rz").is_ok());
    }

    #[test]
    fn corpus_const_valid_simple() {
        let src = "const X = 42;\nprintln(to_string(X));\n";
        let r = run_program(src);
        assert!(r.ok, "{:?}", r.errors);
    }

    #[test]
    fn corpus_const_valid_concat() {
        let src = "const MSG = \"Hi\" + \" there\";\nprintln(MSG);\n";
        let r = run_program(src);
        assert!(r.ok, "{:?}", r.errors);
    }

    #[test]
    fn corpus_const_valid_conditional() {
        let src = "const X = if true { 10 } else { 20 };\nprintln(to_string(X));\n";
        let r = run_program(src);
        assert!(r.ok, "{:?}", r.errors);
    }

    #[test]
    fn corpus_const_malformed_circular() {
        let src = "const X = X;\n";
        let r = run_program(src);
        assert!(
            !r.ok && r.errors.iter().any(|e| e.contains("circular")),
            "{:?}",
            r.errors
        );
    }

    #[test]
    fn corpus_const_malformed_undefined() {
        let src = "const X = UNDEF;\n";
        let r = run_program(src);
        assert!(
            !r.ok && r.errors.iter().any(|e| e.contains("not a compile-time constant")),
            "{:?}",
            r.errors
        );
    }

    #[test]
    fn corpus_const_malformed_mixed_types() {
        let src = "const R = 42 + \"str\";\n";
        let r = run_program(src);
        assert!(!r.ok, "{:?}", r.errors);
    }

    #[test]
    fn corpus_const_malformed_duplicate() {
        // Note: Resilient allows const redefinition; the last definition wins.
        // This test validates that redefined consts compile successfully and use final value.
        let src = "const X = 1;\nconst X = 2;\nprintln(to_string(X));";
        let r = run_program(src);
        assert!(r.ok && r.stdout.contains("2"), "last const definition should be used, got: {:?}", r.stdout);
    }

    #[test]
    fn corpus_const_malformed_var_ref() {
        let src = "let v = 10;\nconst X = v;\n";
        let r = run_program(src);
        assert!(!r.ok, "{:?}", r.errors);
    }

    #[test]
    fn corpus_const_valid_bitwise() {
        let src = "const M = 0xFF & 0x0F;\nprintln(to_string(M));\n";
        let r = run_program(src);
        assert!(r.ok, "{:?}", r.errors);
    }

    #[test]
    fn corpus_const_valid_tuple() {
        let src = "const P = (1, 2);\nlet (a, b) = P;\nprintln(to_string(a + b));\n";
        let r = run_program(src);
        assert!(r.ok, "{:?}", r.errors);
    }
}
