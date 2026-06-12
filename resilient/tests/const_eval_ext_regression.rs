use resilient::run_program;

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
    let err = run_expect_err("const = 1;");
    assert_eq!(
        err,
        "test.rz:1:1: error: invalid const declaration: missing name"
    );
}

#[test]
fn const_decl_missing_initializer_is_rejected() {
    let err = run_expect_err("const ANSWER;");
    assert_eq!(
        err,
        "test.rz:1:1: error: invalid const declaration: missing initializer"
    );
}

#[test]
fn annotated_const_decl_without_initializer_is_rejected() {
    let err = run_expect_err("const VALUE: int;");
    assert_eq!(
        err,
        "test.rz:1:1: error: invalid const declaration: type annotations require an initializer"
    );
}

#[test]
fn const_decl_check_accepts_valid_baselines() {
    let zero = run(r#"
const ANSWER = 42;
println(to_string(ANSWER));
"#);
    assert!(zero.contains("42"), "got: {zero:?}");

    let typed = run(r#"
const LIMIT: int = 7;
println(to_string(LIMIT));
"#);
    assert!(typed.contains("7"), "got: {typed:?}");

    let multiple = run(r#"
const ZERO = 0;
const FOUR = 4;
println(to_string(ZERO + FOUR));
"#);
    assert!(multiple.contains("4"), "got: {multiple:?}");
}

#[test]
fn const_decl_check_rejects_malformed_regressions() {
    let cases = [
        (
            "missing name",
            "const = 1;",
            "test.rz:1:1: error: invalid const declaration: missing name",
        ),
        (
            "whitespace-only name",
            "const \t= 2;",
            "test.rz:1:1: error: invalid const declaration: missing name",
        ),
        (
            "missing initializer",
            "const COUNT;",
            "test.rz:1:1: error: invalid const declaration: missing initializer",
        ),
        (
            "typed missing initializer",
            "const VALUE: int;",
            "test.rz:1:1: error: invalid const declaration: type annotations require an initializer",
        ),
        (
            "duplicate malformed forms",
            "const = 1;\nconst = 2;\n",
            "test.rz:1:1: error: invalid const declaration: missing name",
        ),
    ];

    for (label, src, expected) in cases {
        let err = run_expect_err(src);
        assert_eq!(err, expected, "case {label}");
    }
}
