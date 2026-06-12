use crate::Node;
use crate::const_eval_ext::check;
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

fn program_with(stmts: Vec<Spanned<Node>>) -> Node {
    Node::Program(stmts)
}

fn assert_check_ok(program: Node) {
    assert!(check(&program, "test.rz").is_ok());
}

fn assert_check_err(program: Node, expected: &str, label: &str) {
    let err = check(&program, "test.rz").unwrap_err();
    assert_eq!(err, expected, "case {label}");
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
const A = 0b1100;
const B = 0b1010;
const C = A ^ B;
println(to_string(C));
"#);
    assert!(out.contains("6"), "got: {out:?}");
}

#[test]
fn const_shift() {
    let out = run(r#"
const VALUE = 1 << 4;
println(to_string(VALUE));
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

#[test]
fn const_decl_check_accepts_valid_baselines() {
    assert_check_ok(program(const_stmt(
        "ANSWER",
        Node::IntegerLiteral {
            value: 42,
            span: span(1, 14),
        },
        None,
        1,
        1,
    )));

    assert_check_ok(program(const_stmt(
        "LIMIT",
        Node::IntegerLiteral {
            value: 7,
            span: span(4, 18),
        },
        Some("int"),
        4,
        3,
    )));

    assert_check_ok(program_with(vec![
        const_stmt(
            "ZERO",
            Node::IntegerLiteral {
                value: 0,
                span: span(8, 16),
            },
            None,
            8,
            1,
        ),
        const_stmt(
            "FOUR",
            Node::IntegerLiteral {
                value: 4,
                span: span(9, 15),
            },
            None,
            9,
            1,
        ),
    ]));
}

#[test]
fn const_decl_check_rejects_malformed_regressions() {
    let cases = [
        (
            "missing name",
            program(const_stmt(
                "",
                Node::IntegerLiteral {
                    value: 1,
                    span: span(1, 18),
                },
                None,
                1,
                1,
            )),
            "test.rz:1:1: error: invalid const declaration: missing name",
        ),
        (
            "whitespace-only name",
            program(const_stmt(
                " \t",
                Node::IntegerLiteral {
                    value: 2,
                    span: span(2, 17),
                },
                None,
                2,
                4,
            )),
            "test.rz:2:4: error: invalid const declaration: missing name",
        ),
        (
            "missing initializer",
            program(const_stmt(
                "COUNT",
                Node::IntegerLiteral {
                    value: 0,
                    span: Span::default(),
                },
                None,
                3,
                5,
            )),
            "test.rz:3:5: error: invalid const declaration: missing initializer",
        ),
        (
            "typed missing initializer",
            program(const_stmt(
                "VALUE",
                Node::IntegerLiteral {
                    value: 0,
                    span: Span::default(),
                },
                Some("int"),
                7,
                2,
            )),
            "test.rz:7:2: error: invalid const declaration: type annotations require an initializer",
        ),
        (
            "duplicate malformed forms",
            program_with(vec![
                const_stmt(
                    " ",
                    Node::IntegerLiteral {
                        value: 0,
                        span: Span::default(),
                    },
                    None,
                    11,
                    3,
                ),
                const_stmt(
                    " ",
                    Node::IntegerLiteral {
                        value: 0,
                        span: Span::default(),
                    },
                    None,
                    14,
                    1,
                ),
            ]),
            "test.rz:11:3: error: invalid const declaration: missing name",
        ),
    ];

    for (label, program, expected) in cases {
        assert_check_err(program, expected, label);
    }
}
