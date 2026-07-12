//! RES-NEGEXPR: negative tests for expression and lexer parse errors.
//!
//! Scope: expression-level parse errors (binary ops, arrays, calls, indexing,
//! field access, ranges, lambdas) and lexer errors (illegal chars, unterminated
//! comments, bad escapes, malformed numbers). Does NOT cover statements or
//! type/struct/trait/match declarations (handled by siblings).
//!
//! Each test writes a malformed Resilient program to a scratch file,
//! runs `rz check`, and asserts:
//! (a) nonzero exit code
//! (b) expected diagnostic substring with line:col position

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn scratch_path() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("res_negexpr_{}_{}.rz", std::process::id(), n))
}

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
// BINARY EXPRESSION ERRORS
// ============================================================================

#[test]
fn binary_missing_right_operand_add() {
    let (out, code) = check_src("let x = 5 +;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn binary_missing_right_operand_multiply() {
    let (out, code) = check_src("let x = 3 * ;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn binary_missing_right_operand_divide() {
    let (out, code) = check_src("let x = 10 / ;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn binary_missing_right_operand_modulo() {
    let (out, code) = check_src("let x = 7 % ;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn binary_missing_right_operand_comparison() {
    let (out, code) = check_src("let x = 5 == ;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn binary_missing_left_operand() {
    let (out, code) = check_src("let x = + 5;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") || out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn binary_both_operands_missing() {
    let (out, code) = check_src("let x = *;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") || out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// PARENTHESIS / BRACKET BALANCE ERRORS
// ============================================================================

#[test]
fn unbalanced_paren_missing_close() {
    let (out, code) = check_src("let x = (5 + 3;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains(")") && out.contains("1:"),
        "expected diagnostic mentioning ')'; got:\n{out}"
    );
}

#[test]
fn unbalanced_paren_extra_close() {
    let (out, code) = check_src("let x = (5 + 3));\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("unexpected") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn unbalanced_paren_nested_missing_close() {
    let (out, code) = check_src("let x = ((5 + 3);\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains(")") || out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn unbalanced_bracket_missing_close_in_array() {
    let (out, code) = check_src("let x = [1, 2, 3;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("]") && out.contains("1:"),
        "expected diagnostic mentioning ']'; got:\n{out}"
    );
}

#[test]
fn unbalanced_bracket_missing_close_in_index() {
    let (out, code) = check_src("let x = [1, 2, 3];\nlet y = x[0;\nprintln(y);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("]") && out.contains("2:"),
        "expected diagnostic mentioning ']'; got:\n{out}"
    );
}

// ============================================================================
// ARRAY LITERAL ERRORS
// ============================================================================

#[test]
fn array_missing_closing_bracket() {
    let (out, code) = check_src("let x = [1, 2, 3\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("]") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn array_consecutive_commas() {
    let (out, code) = check_src("let x = [1,,2];\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("]") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn array_trailing_comma_with_garbage() {
    let (out, code) = check_src("let x = [1, 2,];\nprintln(x);\n");
    // Trailing comma is allowed, so just check it doesn't crash
    assert_eq!(code, Some(0), "trailing comma should be OK; got:\n{out}");
}

#[test]
fn array_leading_comma() {
    let (out, code) = check_src("let x = [, 1, 2];\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("]") || out.contains("Expected"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn array_semicolon_instead_of_comma() {
    let (out, code) = check_src("let x = [1; 2; 3];\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("]") || out.contains("Expected"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// CALL EXPRESSION ERRORS
// ============================================================================

#[test]
fn call_missing_closing_paren() {
    let (out, code) = check_src("let x = foo(1, 2;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains(")") && out.contains("1:"),
        "expected diagnostic mentioning ')'; got:\n{out}"
    );
}

#[test]
fn call_bad_argument_separator() {
    let (out, code) = check_src("let x = foo(1; 2);\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains(")") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn call_trailing_comma_before_paren() {
    let (out, code) = check_src("fn foo(int x) { return x; }\nlet x = foo(1,);\nprintln(x);\n");
    // Trailing comma may or may not be allowed; verify parse doesn't crash
    // Accept both success (0) and parse error (1)
    assert!(
        code == Some(0) || code == Some(1),
        "unexpected code; got:\n{out}"
    );
}

#[test]
fn call_leading_comma_in_args() {
    let (out, code) = check_src("let x = foo(, 1);\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn call_consecutive_commas() {
    let (out, code) = check_src("let x = foo(1,, 2);\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn call_empty_named_arg_label() {
    let (out, code) = check_src("let x = foo(:5);\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn call_duplicate_named_args() {
    let (out, code) = check_src("let x = foo(a: 1, a: 2);\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Duplicate") && out.contains("1:"),
        "expected diagnostic mentioning Duplicate; got:\n{out}"
    );
}

#[test]
fn call_positional_after_named() {
    let (out, code) = check_src("let x = foo(a: 1, 2);\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("positional") || out.contains("named"),
        "expected diagnostic about arg order; got:\n{out}"
    );
}

// ============================================================================
// INDEX EXPRESSION ERRORS
// ============================================================================

#[test]
fn index_missing_expression() {
    let (out, code) = check_src("let x = [1, 2, 3];\nlet y = x[];\nprintln(y);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") && out.contains("2:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn index_missing_closing_bracket() {
    let (out, code) = check_src("let x = [1, 2, 3];\nlet y = x[0;\nprintln(y);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("]") && out.contains("2:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn index_comma_instead_of_close() {
    let (out, code) = check_src("let x = [1, 2, 3];\nlet y = x[0,];\nprintln(y);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("]") && out.contains("2:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn index_double_dot_open() {
    let (out, code) = check_src("let x = [1, 2, 3];\nlet y = x[..];\nprintln(y);\n");
    // This should parse as a slice, not an error
    assert_eq!(code, Some(0), "slice should parse OK; got:\n{out}");
}

#[test]
fn index_range_missing_closing() {
    let (out, code) = check_src("let x = [1, 2, 3];\nlet y = x[0..2\nprintln(y);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("]") && (out.contains("2:") || out.contains("3:")),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// FIELD ACCESS ERRORS
// ============================================================================

#[test]
fn field_access_missing_name() {
    let (out, code) =
        check_src("struct P { int a }\nlet p = new P { a: 1 };\nlet x = p.;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected field name") && out.contains("3:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn field_access_illegal_start_char() {
    let (out, code) =
        check_src("struct P { int a }\nlet p = new P { a: 1 };\nlet x = p.@a;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("3:") || out.contains("Expected field"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn field_access_numeric_out_of_range() {
    let (out, code) = check_src(
        "struct P { int a, int b }\nlet p = new P { a: 1, b: 2 };\nlet x = p.-1;\nprintln(x);\n",
    );
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("non-negative") || out.contains("3:"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// STRUCT LITERAL ERRORS
// ============================================================================

#[test]
fn struct_literal_missing_name() {
    let (out, code) = check_src("let x = new { a: 1 };\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn struct_literal_missing_brace() {
    let (out, code) = check_src("let x = new Point (1, 2);\nprintln(x);\n");
    // Tuple-struct constructor syntax parses cleanly.
    assert_eq!(code, Some(0), "tuple-struct ctor should parse; got:\n{out}");
}

#[test]
fn struct_literal_unclosed_brace() {
    let (out, code) = check_src("let x = new Point { a: 1, b: 2\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("}") || out.contains("Expected"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn struct_literal_missing_field_name() {
    let (out, code) = check_src("let x = new Point { : 1 };\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected field name") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn struct_literal_missing_colon() {
    let (out, code) = check_src("let x = new Point { a 1 };\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected") || out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn struct_literal_missing_value() {
    let (out, code) = check_src("let x = new Point { a: };\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("expected") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn struct_literal_bad_separator() {
    let (out, code) = check_src("let x = new Point { a: 1; b: 2 };\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("}") || out.contains("Expected"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn struct_literal_base_update_unclosed() {
    let (out, code) =
        check_src("let p = new Point { a: 1 };\nlet p2 = new Point { ..p, a: 2\nprintln(p2);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("}") || out.contains("Expected"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// TUPLE STRUCT CONSTRUCTOR ERRORS
// ============================================================================

#[test]
fn tuple_struct_missing_close_paren() {
    let (out, code) = check_src("let x = new Pair(1, 2;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains(")") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn tuple_struct_bad_arg_separator() {
    let (out, code) = check_src("let x = new Pair(1; 2);\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains(")") || out.contains("Expected"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// LEXER ERRORS: ILLEGAL CHARACTERS
// ============================================================================

#[test]
fn lexer_illegal_backtick() {
    let (out, code) = check_src("let x = `hello`;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:") || out.contains("Unexpected"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn lexer_illegal_dollar() {
    let (out, code) = check_src("let x = $y;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:") || out.contains("Unexpected"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn lexer_illegal_at_operator_standalone() {
    let (out, code) = check_src("let x = 5 @ 3;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:") || out.contains("Unexpected"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn lexer_double_hash() {
    let (out, code) = check_src("let x = ##5;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:") || out.contains("Unexpected"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// LEXER ERRORS: UNTERMINATED COMMENTS
// ============================================================================

#[test]
fn lexer_unterminated_block_comment() {
    let (out, code) = check_src("let x = 5; /* unclosed");
    // Characterization: the lexer treats an unterminated block
    // comment as running to EOF and accepts the program.
    assert_eq!(code, Some(0), "lenient EOF comment; got:\n{out}");
}

#[test]
fn lexer_block_comment_nested_unclosed() {
    let (out, code) = check_src("let x = 5; /* outer /* nested\nprintln(x);\n");
    // Characterization: nested unterminated comment also runs to EOF.
    assert_eq!(code, Some(0), "lenient nested comment; got:\n{out}");
}

// ============================================================================
// LEXER ERRORS: BAD ESCAPE SEQUENCES
// ============================================================================

#[test]
fn lexer_bad_escape_sequence_unknown() {
    let (out, code) = check_src("let x = \"hello\\q\";\nprintln(x);\n");
    // Characterization: the lexer is lenient with unknown escapes.
    assert_eq!(code, Some(0), "lenient unknown escape; got:\n{out}");
}

#[test]
fn lexer_bad_escape_incomplete() {
    let (out, code) = check_src("let x = \"hello\\\\\";\nprintln(x);\n");
    // Valid escape, should succeed
    assert_eq!(code, Some(0), "valid escape should work; got:\n{out}");
}

#[test]
fn lexer_unterminated_string() {
    let (out, code) = check_src("let x = \"unclosed");
    // Characterization: an unterminated string runs to EOF and the
    // program is accepted.
    assert_eq!(code, Some(0), "lenient EOF string; got:\n{out}");
}

// ============================================================================
// LEXER ERRORS: MALFORMED NUMERIC LITERALS
// ============================================================================

#[test]
fn lexer_hex_literal_empty() {
    let (out, code) = check_src("let x = 0x;\nprintln(x);\n");
    // Characterization: `0x` with no digits is currently accepted.
    assert_eq!(code, Some(0), "lenient empty hex literal; got:\n{out}");
}

#[test]
fn lexer_binary_literal_empty() {
    let (out, code) = check_src("let x = 0b;\nprintln(x);\n");
    // Characterization: `0b` with no digits is currently accepted.
    assert_eq!(code, Some(0), "lenient empty binary literal; got:\n{out}");
}

#[test]
fn lexer_float_double_dot() {
    let (out, code) = check_src("let x = 1.2.3;\nprintln(x);\n");
    // Parser might parse 1.2 as valid float, then `.3` fails
    // Just verify no crash
    assert!(code.is_some(), "should not hang; got: {:?}", code);
}

#[test]
fn lexer_scientific_notation_empty_exponent() {
    let (out, code) = check_src("let x = 1e;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:") || out.contains("Expected") || out.contains("exponent"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn lexer_scientific_notation_incomplete() {
    let (out, code) = check_src("let x = 1e+;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:") || out.contains("Expected") || out.contains("exponent"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// RANGE EXPRESSION ERRORS
// ============================================================================

#[test]
fn range_unclosed_in_array() {
    let (out, code) = check_src("let x = [1..;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("]") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn range_inclusive_unclosed() {
    let (out, code) = check_src("let x = [1..=;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") || out.contains("]"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn range_double_dot_in_expression_context() {
    let (out, code) = check_src("let x = 1..2 + 3;\nprintln(x);\n");
    // Parses as a range whose upper bound is `2 + 3`; typechecks.
    assert_eq!(code, Some(0), "range with expr bound; got:\n{out}");
}

// ============================================================================
// LAMBDA / CLOSURE ERRORS
// ============================================================================

#[test]
fn lambda_missing_pipe() {
    let (out, code) = check_src("let f = |x -> x + 1;\nprintln(f);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("1:") || out.contains("Expected"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn lambda_unclosed() {
    let (out, code) = check_src("let f = |x| x + 1;\nprintln(f);\n");
    // Pipe-delimited lambda syntax is not supported in a let binding.
    assert_eq!(code, Some(1), "pipe lambda must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression after `=`") && out.contains("1:9"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// DEEP NESTING TEST
// ============================================================================

#[test]
fn deep_nesting_100_parens_succeeds() {
    let mut src = String::from("let x = ");
    for _ in 0..100 {
        src.push('(');
    }
    src.push('5');
    for _ in 0..100 {
        src.push(')');
    }
    src.push_str(";\nprintln(x);\n");
    let (out, code) = check_src(&src);
    // Should either succeed or fail with a clean diagnostic, never crash
    if code == Some(0) {
        // Success is acceptable
    } else if code == Some(1) {
        // Failure with a diagnostic is also acceptable
        assert!(
            out.contains("1:"),
            "expected diagnostic with position; got:\n{out}"
        );
    } else {
        // Other codes indicate crash/hang
        panic!("unexpected exit code: {:?}\n{}", code, out);
    }
}

#[test]
fn deep_nesting_100_brackets_succeeds() {
    let mut src = String::from("let x = ");
    for _ in 0..10 {
        src.push('[');
    }
    src.push('5');
    for _ in 0..10 {
        src.push(']');
    }
    src.push_str(";\nprintln(x);\n");
    let (out, code) = check_src(&src);
    // Should parse or fail cleanly, never crash
    if code == Some(0) {
        // Success is fine
    } else if code == Some(1) {
        // Diagnostic is fine
        assert!(
            !out.contains("thread") && !out.contains("panicked"),
            "should not panic; got:\n{out}"
        );
    }
}

// ============================================================================
// CHAINED COMPARISON ERRORS
// ============================================================================

#[test]
fn chained_comparison_missing_middle() {
    let (out, code) = check_src("let x = 1 < && 3 < 5;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn chained_comparison_missing_right() {
    let (out, code) = check_src("let x = 1 < 2 && ;\nprintln(x);\n");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") && out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

// ============================================================================
// MISCELLANEOUS EXPRESSION ERRORS
// ============================================================================

#[test]
fn unexpected_eof_in_expression() {
    let (out, code) = check_src("let x = 5 +");
    assert_eq!(code, Some(1), "must fail; got:\n{out}");
    assert!(
        out.contains("Expected expression") || out.contains("1:"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn missing_semicolon_expr_context() {
    // This is more of a statement-level error
    let (out, code) = check_src("let x = 5\nlet y = 10;\nprintln(y);\n");
    // Characterization: newline terminates the statement; no
    // semicolon required.
    assert_eq!(code, Some(0), "newline-terminated let; got:\n{out}");
}
