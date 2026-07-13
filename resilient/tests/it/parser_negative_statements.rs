//! RES-NEGSTMT: negative tests for statement/declaration-level parse errors.
//!
//! Each test feeds a tiny malformed Resilient program to the real `rz check`
//! binary and asserts:
//! (a) nonzero exit code
//! (b) a specific diagnostic substring (including line:col position)
//!
//! This harness follows the pattern from index_typecheck_smoke.rs:
//! scratch file + Command::new(env!("CARGO_BIN_EXE_rz")) + "check" +
//! combined stdout/stderr.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

/// Build a unique scratch path inside the OS temp dir.
fn scratch_path() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("res_negstmt_{}_{}.rz", std::process::id(), n))
}

/// Run `rz check <file>` on a piece of Resilient source and return
/// (combined stdout+stderr, exit code).
fn check_src(src: &str) -> (String, Option<i32>) {
    let path = scratch_path();
    std::fs::write(&path, src).expect("write scratch");
    let out = Command::new(bin())
        .arg("check")
        .arg(&path)
        .output()
        .expect("spawn rz check");
    let _ = std::fs::remove_file(&path);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (combined, out.status.code())
}

// ============================================================================
// LET STATEMENT ERRORS
// ============================================================================

#[test]
fn let_missing_identifier() {
    let (out, code) = check_src("let = 5;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected identifier after 'let'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn let_missing_equals() {
    let (out, code) = check_src("let x 5;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected '=' after identifier") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn let_missing_value_expression() {
    let (out, code) = check_src("let x = ;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression after `=`") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn let_with_type_annotation_missing_type() {
    let (out, code) = check_src("let x: = 5;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

#[test]
fn let_tuple_destructure_missing_closing_paren() {
    let (out, code) = check_src("let (a, b = 5;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

// ============================================================================
// FUNCTION DECLARATION ERRORS
// ============================================================================

#[test]
fn fn_missing_name() {
    let (out, code) = check_src("fn (int x) { return 0; }\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected identifier after 'fn'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn fn_missing_open_paren() {
    let (out, code) = check_src("fn foo int x { return 0; }\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected '('") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn fn_missing_close_paren() {
    let (out, code) = check_src("fn foo(int x { return 0; }\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

#[test]
fn fn_missing_open_brace() {
    let (out, code) = check_src("fn foo(int x) return 0;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected '{'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn fn_missing_close_brace() {
    let (out, code) = check_src("fn foo(int x) { return 0;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

#[test]
fn fn_param_missing_type() {
    let (out, code) = check_src("fn foo(x) { return 0; }\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

#[test]
fn fn_param_syntax_error() {
    let (out, code) = check_src("fn foo(int x int y) { return 0; }\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

// ============================================================================
// IF STATEMENT ERRORS
// ============================================================================

#[test]
fn if_missing_open_paren() {
    let (out, code) = check_src("if x > 0 { }\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

#[test]
fn if_missing_close_paren() {
    let (out, code) = check_src("if (x > 0 { }\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected ')' after if condition") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn if_missing_open_brace() {
    let (out, code) = check_src("if (x > 0) return 0;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected '{'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn if_missing_close_brace() {
    let (out, code) = check_src("if (x > 0) { return 0;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

#[test]
fn if_else_missing_brace() {
    let (out, code) = check_src("if (x > 0) { } else return 0;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected '{'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// WHILE STATEMENT ERRORS
// ============================================================================

#[test]
fn while_missing_close_paren() {
    let (out, code) = check_src("fn main(int x) { while (x > 0 { } }\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected ')' after while condition") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn while_missing_open_brace() {
    let (out, code) = check_src("fn main(int x) { while (x > 0) x = x - 1; }\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected '{'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn while_with_missing_body() {
    let (out, code) = check_src("fn main(int x) { while (x > 0) }\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected '{'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// FOR LOOP ERRORS
// ============================================================================

#[test]
fn for_missing_identifier() {
    let (out, code) = check_src("fn main(int x) { for in [1, 2] { } }\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected identifier after 'for'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn for_missing_in_keyword() {
    let (out, code) = check_src("fn main(int x) { for i [1, 2] { } }\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected 'in' after 'for") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn for_missing_iterable() {
    let (out, code) = check_src("fn main(int x) { for i in { } }\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

#[test]
fn for_missing_open_brace() {
    let (out, code) = check_src("fn main(int x) { for i in [1, 2] x = i; }\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected '{'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn for_missing_body_brace() {
    let (out, code) = check_src("fn main(int x) { for i in [1, 2] }\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected '{'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// BLOCK ERRORS
// ============================================================================

#[test]
fn struct_missing_open_brace() {
    let (out, code) = check_src("struct Point let x = 5;\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

#[test]
fn invalid_keyword_combo() {
    let (out, code) = check_src("const mut x = 5;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

// ============================================================================
// TYPE ALIAS ERRORS
// ============================================================================

#[test]
fn type_alias_missing_name() {
    let (out, code) = check_src("type = int;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected alias name after 'type'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn type_alias_missing_equals() {
    let (out, code) = check_src("type MyInt int;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected '=' after 'type") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// UNSAFE BLOCK ERRORS
// ============================================================================

#[test]
fn unsafe_missing_brace() {
    let (out, code) = check_src("fn main(int x) { unsafe let y = 5; }\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected '{' after 'unsafe'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// NUMERIC LITERAL ERRORS
// ============================================================================

#[test]
fn struct_field_missing_type() {
    let (out, code) = check_src("struct Point { x, y }\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

#[test]
fn invalid_hex_literal() {
    let (out, code) = check_src("let x = 0xG;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

// ============================================================================
// USE STATEMENT ERRORS (optional — if lexer handles them)
// ============================================================================

#[test]
fn use_missing_path() {
    let (out, code) = check_src("use ;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

// ============================================================================
// MATCH STATEMENT ERRORS (minimal parser-level tests)
// ============================================================================

#[test]
fn match_missing_fat_arrow() {
    let (out, code) = check_src("fn main(int x) { match x { _ 0 } }\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

// ============================================================================
// ATTRIBUTE ERRORS
// ============================================================================

#[test]
fn pub_without_valid_target() {
    let (out, code) = check_src("pub let x = 5;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        (out.contains("'pub' must be followed by") || out.contains("followed by"))
            && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// IMPLEMENTATION BLOCK ERRORS
// ============================================================================

#[test]
fn impl_missing_name() {
    let (out, code) = check_src("impl { }\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected struct or trait name after 'impl'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn impl_missing_open_brace() {
    let (out, code) = check_src("impl MyStruct }\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected '{'") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn impl_missing_close_brace() {
    let (out, code) = check_src("impl MyStruct { fn foo(int x) { } \nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

// ============================================================================
// ASSIGNMENT ERRORS
// ============================================================================

#[test]
fn field_access_on_literal() {
    let (out, code) = check_src("let x = 5.foo;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

#[test]
fn invalid_method_call() {
    let (out, code) = check_src("let x = 5.();\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

// ============================================================================
// MISCELLANEOUS ERRORS
// ============================================================================

#[test]
fn array_missing_closing_bracket() {
    let (out, code) = check_src("let x = [1, 2, 3;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}

#[test]
fn return_outside_function() {
    let (out, code) = check_src("let x = return 5;\nprintln(1);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:"),
        "expected line position in diagnostic; got:\n{out}"
    );
}
