//! Negative tests for parser errors in types, structs, traits, and matches.
//!
//! Scope: parse-level errors in type annotations, struct/enum declarations,
//! impl blocks, trait definitions, and match expressions. Does NOT cover
//! let/fn/control-flow statement errors (handled by a sibling agent).
//!
//! Each test writes a small Resilient program to a scratch file with the
//! prefix "res_negtype", runs `rz check` against it, and asserts:
//! - exit code is Some(1) (failure)
//! - stdout+stderr contains the expected error substring with line:col position

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn check_src(src: &str) -> (String, Option<i32>) {
    static CTR: AtomicUsize = AtomicUsize::new(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf =
        std::env::temp_dir().join(format!("res_negtype_{}_{}.rz", std::process::id(), n));
    std::fs::write(&path, src).expect("write tmp");
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

// ========== Type Annotation Errors ==========

#[test]
fn type_annot_missing_closing_angle_in_generic() {
    let (out, code) = check_src("fn foo<T(int a : T { return a; }\nfoo(5);\n");
    assert_eq!(
        code,
        Some(1),
        "missing closing angle must fail; got: {}",
        out
    );
    assert!(
        out.contains("Expected") && out.contains(">"),
        "expected diagnostic mentioning '>'; got: {}",
        out
    );
}

#[test]
fn type_annot_missing_closing_paren_in_fn_type() {
    let (out, code) = check_src("fn id(fn(int -> int x) { return x; }\nid(nil);\n");
    assert_eq!(
        code,
        Some(1),
        "missing closing paren in fn type must fail; got: {}",
        out
    );
    assert!(
        out.contains(")") || out.contains("closing"),
        "expected diagnostic mentioning paren; got: {}",
        out
    );
}

#[test]
fn type_annot_wrong_separator_in_generic_list() {
    let (out, code) = check_src("fn foo<T; U>(T a, U b) { return a; }\nfoo(1, 2);\n");
    assert_eq!(
        code,
        Some(1),
        "semicolon in type params must fail; got: {}",
        out
    );
    assert!(
        out.contains(">") || out.contains(","),
        "expected diagnostic about separator; got: {}",
        out
    );
}

#[test]
fn type_annot_missing_comma_in_generic_list() {
    let (out, code) = check_src("fn foo<T U>(T a, U b) { return a; }\nfoo(1, 2);\n");
    assert_eq!(
        code,
        Some(1),
        "missing comma in type params must fail; got: {}",
        out
    );
    assert!(
        out.contains(",") || out.contains(">"),
        "expected diagnostic about comma or angle; got: {}",
        out
    );
}

#[test]
fn type_annot_missing_bound_name_after_colon() {
    let (out, code) = check_src("fn foo<T : +U>(T a) { return a; }\nfoo(5);\n");
    assert_eq!(
        code,
        Some(1),
        "missing trait name after colon must fail; got: {}",
        out
    );
    assert!(
        out.contains("trait") || out.contains("bound"),
        "expected diagnostic about trait; got: {}",
        out
    );
}

#[test]
fn type_annot_missing_closing_bracket_in_array_type() {
    let (out, code) = check_src("let x: [int; 5 = [1, 2, 3, 4, 5];\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "missing bracket in array type must fail; got: {}",
        out
    );
    assert!(
        out.contains("]") || out.contains("closing"),
        "expected diagnostic about bracket; got: {}",
        out
    );
}

#[test]
fn type_annot_missing_semicolon_in_array_type() {
    let (out, code) = check_src("fn foo(int[5 10]) { return 0; }\nfoo(0);\n");
    assert_eq!(
        code,
        Some(1),
        "missing semicolon in array type must fail; got: {}",
        out
    );
    assert!(
        out.contains(";") || out.contains("array"),
        "expected diagnostic about semicolon; got: {}",
        out
    );
}

#[test]
fn type_annot_wrong_separator_in_fn_params() {
    let (out, code) = check_src("fn foo(fn(int; int -> int) { return 0; }\nfoo(nil);\n");
    assert_eq!(
        code,
        Some(1),
        "semicolon in fn type params must fail; got: {}",
        out
    );
    assert!(
        out.contains(",") || out.contains(")"),
        "expected diagnostic about separator; got: {}",
        out
    );
}

// ========== Struct Declaration Errors ==========

#[test]
fn struct_decl_missing_opening_brace() {
    let (out, code) =
        check_src("struct Point int x int y\nlet p = new Point { x: 1, y: 2 };\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "missing opening brace in struct decl must fail; got: {}",
        out
    );
    assert!(
        out.contains("{") || out.contains("after struct"),
        "expected diagnostic about brace; got: {}",
        out
    );
}

#[test]
fn struct_decl_missing_closing_brace() {
    let (out, code) =
        check_src("struct Point { int x, int y\nlet p = new Point { x: 1 };\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "missing closing brace in struct decl must fail; got: {}",
        out
    );
    assert!(
        out.contains("}") || out.contains("closing"),
        "expected diagnostic about brace; got: {}",
        out
    );
}

#[test]
fn struct_decl_missing_field_type() {
    let (out, code) =
        check_src("struct Point { x int y }\nlet p = new Point { x: 1, y: 2 };\nprint(0);\n");
    assert_eq!(code, Some(1), "missing field type must fail; got: {}", out);
    assert!(
        out.contains("type") || out.contains("field"),
        "expected diagnostic about field type; got: {}",
        out
    );
}

#[test]
fn struct_decl_missing_field_name() {
    let (out, code) =
        check_src("struct Point { int int y }\nlet p = new Point { y: 1 };\nprint(0);\n");
    assert_eq!(code, Some(1), "missing field name must fail; got: {}", out);
    assert!(
        out.contains("name") || out.contains("field"),
        "expected diagnostic about field name; got: {}",
        out
    );
}

#[test]
fn struct_decl_duplicate_field_name() {
    let (out, code) =
        check_src("struct Point { int x, int x }\nlet p = new Point { x: 1 };\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "duplicate field name must fail; got: {}",
        out
    );
    assert!(
        out.contains("Duplicate") || out.contains("duplicate"),
        "expected diagnostic about duplicate; got: {}",
        out
    );
}

#[test]
fn struct_decl_wrong_separator() {
    let (out, code) =
        check_src("struct Point { int x; int y }\nlet p = new Point { x: 1, y: 2 };\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "semicolon instead of comma in struct must fail; got: {}",
        out
    );
    assert!(
        out.contains(",") || out.contains("}"),
        "expected diagnostic about separator; got: {}",
        out
    );
}

#[test]
fn struct_decl_trailing_garbage() {
    let (out, code) = check_src(
        "struct Point { int x, int y } foo\nlet p = new Point { x: 1, y: 2 };\nprint(0);\n",
    );
    assert_eq!(
        code,
        Some(1),
        "trailing garbage in struct must fail; got: {}",
        out
    );
    assert!(
        out.contains("error") || out.contains("Expected"),
        "expected diagnostic about trailing garbage; got: {}",
        out
    );
}

// ========== Struct Literal Errors ==========

#[test]
fn struct_literal_missing_field_value() {
    let (out, code) =
        check_src("struct Point { int x, int y }\nlet p = new Point { x: , y: 2 };\nprint(0);\n");
    assert_eq!(code, Some(1), "missing field value must fail; got: {}", out);
    assert!(
        out.contains("value") || out.contains("Expected") || out.contains(","),
        "expected diagnostic about missing value; got: {}",
        out
    );
}

#[test]
fn struct_literal_unknown_field() {
    let (out, code) =
        check_src("struct Point { int x, int y }\nlet p = new Point { z: 5 };\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "unknown field in literal must fail; got: {}",
        out
    );
    assert!(
        out.contains("unknown") || out.contains("field") || out.contains("error"),
        "expected diagnostic about unknown field; got: {}",
        out
    );
}

#[test]
fn struct_literal_missing_closing_brace() {
    let (out, code) =
        check_src("struct Point { int x, int y }\nlet p = new Point { x: 1, y: 2;\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "missing closing brace in struct literal must fail; got: {}",
        out
    );
    assert!(
        out.contains("}") || out.contains("closing"),
        "expected diagnostic about closing brace; got: {}",
        out
    );
}

// ========== Impl Block Errors ==========

#[test]
fn impl_block_missing_type_name() {
    let (out, code) =
        check_src("struct P { int x }\nimpl { fn foo(self int) { return 0; } }\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "missing type in impl must fail; got: {}",
        out
    );
    assert!(
        out.contains("struct") || out.contains("trait") || out.contains("name"),
        "expected diagnostic about type name; got: {}",
        out
    );
}

#[test]
fn impl_block_missing_method_fn_keyword() {
    let (out, code) =
        check_src("struct P { int x }\nimpl P { foo(self int) { return 0; } }\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "missing 'fn' in impl method must fail; got: {}",
        out
    );
    assert!(
        out.contains("fn") || out.contains("Expected"),
        "expected diagnostic about 'fn'; got: {}",
        out
    );
}

#[test]
fn impl_block_missing_method_name() {
    let (out, code) =
        check_src("struct P { int x }\nimpl P { fn (self int) { return 0; } }\nprint(0);\n");
    assert_eq!(code, Some(1), "missing method name must fail; got: {}", out);
    assert!(
        out.contains("name") || out.contains("identifier"),
        "expected diagnostic about method name; got: {}",
        out
    );
}

// ========== Match Expression Errors ==========

#[test]
fn match_missing_fat_arrow_after_pattern() {
    let (out, code) = check_src("fn f(int x) { return match x { 1 2 }; }\nf(0);\n");
    assert_eq!(
        code,
        Some(1),
        "missing fat arrow after pattern must fail; got: {}",
        out
    );
    assert!(
        out.contains("=>") || out.contains("Expected"),
        "expected diagnostic about fat arrow; got: {}",
        out
    );
}

#[test]
fn match_missing_closing_brace() {
    let (out, code) = check_src("fn f(int x) { return match x { 1 => 2; }; }\nf(0);\n");
    assert_eq!(
        code,
        Some(1),
        "missing closing brace in match must fail; got: {}",
        out
    );
    assert!(
        out.contains("}") || out.contains("closing") || out.contains("Unexpected EOF"),
        "expected diagnostic about closing brace; got: {}",
        out
    );
}

#[test]
fn match_eof_inside_arms() {
    let (out, code) = check_src("fn f(int x) { return match x { 1 => 2\n");
    assert_eq!(code, Some(1), "EOF inside match must fail; got: {}", out);
    assert!(
        out.contains("EOF") || out.contains("Unexpected"),
        "expected diagnostic about EOF; got: {}",
        out
    );
}

// ========== Enum/Sum-Type Declaration Errors ==========

#[test]
fn enum_decl_missing_variant_name() {
    let (out, code) = check_src("enum Color { (int), Green }\nlet c = Green;\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "missing variant name must fail; got: {}",
        out
    );
    assert!(
        out.contains("identifier") || out.contains("Expected"),
        "expected diagnostic about variant name; got: {}",
        out
    );
}

#[test]
fn enum_decl_duplicate_variant() {
    let (out, code) = check_src("enum Color { Red, Red }\nlet c = Red;\nprint(0);\n");
    assert_eq!(code, Some(1), "duplicate variant must fail; got: {}", out);
    assert!(
        out.contains("Duplicate") || out.contains("duplicate"),
        "expected diagnostic about duplicate; got: {}",
        out
    );
}

#[test]
fn enum_decl_missing_closing_brace() {
    let (out, code) = check_src("enum Color { Red, Green\nlet c = Red;\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "missing closing brace in enum must fail; got: {}",
        out
    );
    assert!(
        out.contains("}") || out.contains("closing"),
        "expected diagnostic about closing brace; got: {}",
        out
    );
}

#[test]
fn enum_decl_missing_opening_brace() {
    let (out, code) = check_src("enum Color Red, Green\nlet c = Red;\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "missing opening brace in enum must fail; got: {}",
        out
    );
    assert!(
        out.contains("{") || out.contains("Expected"),
        "expected diagnostic about opening brace; got: {}",
        out
    );
}

// ========== Trait Declaration Errors ==========

#[test]
fn trait_decl_missing_name() {
    let (out, code) = check_src("trait { fn foo(self int); }\nprint(0);\n");
    assert_eq!(code, Some(1), "missing trait name must fail; got: {}", out);
    assert!(
        out.contains("identifier") || out.contains("trait"),
        "expected diagnostic about trait name; got: {}",
        out
    );
}

#[test]
fn trait_decl_missing_closing_brace() {
    let (out, code) = check_src("trait Foo { fn bar(self int); let x = 5;\n");
    assert_eq!(
        code,
        Some(1),
        "missing closing brace in trait must fail; got: {}",
        out
    );
    assert!(
        out.contains("fn") || out.contains("type") || out.contains("Expected"),
        "expected diagnostic about trait body content; got: {}",
        out
    );
}

// ========== Generic Parameter List Errors ==========

#[test]
fn generic_params_invalid_char_in_param() {
    let (out, code) = check_src("fn foo<123>(int x) { return x; }\nfoo(5);\n");
    assert_eq!(
        code,
        Some(1),
        "numeric literal in type params must fail; got: {}",
        out
    );
    assert!(
        out.contains("name") || out.contains("identifier") || out.contains("Expected"),
        "expected diagnostic about param name; got: {}",
        out
    );
}

#[test]
fn generic_params_missing_closing_angle() {
    let (out, code) = check_src("fn foo<T(int x) { return x; }\nfoo(5);\n");
    assert_eq!(
        code,
        Some(1),
        "missing closing angle in type params must fail; got: {}",
        out
    );
    assert!(
        out.contains(">") || out.contains("closing"),
        "expected diagnostic about closing angle; got: {}",
        out
    );
}

#[test]
fn generic_params_extra_colon() {
    let (out, code) = check_src("fn foo<T::U>(T a) { return a; }\nfoo(5);\n");
    assert_eq!(
        code,
        Some(1),
        "extra separator in type params must fail; got: {}",
        out
    );
    assert!(
        out.contains(">") || out.contains("Expected"),
        "expected diagnostic about type params; got: {}",
        out
    );
}

// ========== Pattern Errors in Match ==========

#[test]
fn match_pattern_range_missing_hi() {
    let (out, code) = check_src("fn f(int x) { return match x { 1..a => 2 }; }\nf(0);\n");
    assert_eq!(
        code,
        Some(1),
        "invalid range pattern must fail; got: {}",
        out
    );
    assert!(
        out.contains("integer") || out.contains("literal") || out.contains("upper bound"),
        "expected diagnostic about range; got: {}",
        out
    );
}

#[test]
fn match_pattern_unsupported_start() {
    let (out, code) = check_src("fn f(int x) { return match x { @foo => 2 }; }\nf(0);\n");
    assert_eq!(
        code,
        Some(1),
        "unsupported pattern start must fail; got: {}",
        out
    );
    assert!(
        out.contains("pattern") || out.contains("Unsupported"),
        "expected diagnostic about pattern; got: {}",
        out
    );
}

// ========== Tuple Type Errors ==========

#[test]
fn tuple_literal_missing_comma() {
    let (out, code) = check_src("fn f() { let t = (1 2); return 0; }\nf();\n");
    assert_eq!(
        code,
        Some(1),
        "missing comma in tuple literal must fail; got: {}",
        out
    );
    assert!(
        out.contains(",") || out.contains("tuple"),
        "expected diagnostic about comma; got: {}",
        out
    );
}

#[test]
fn tuple_literal_missing_closing_paren() {
    let (out, code) = check_src("fn f() { let t = (1, 2; return 0; }\nf();\n");
    assert_eq!(
        code,
        Some(1),
        "missing closing paren in tuple literal must fail; got: {}",
        out
    );
    assert!(
        out.contains(")") || out.contains("closing"),
        "expected diagnostic about closing paren; got: {}",
        out
    );
}

// ========== Reference Type Errors ==========

#[test]
fn ref_type_missing_close_bracket_in_region() {
    let (out, code) = check_src("fn foo(&[L int x) { return 0; }\nfoo(nil);\n");
    assert_eq!(
        code,
        Some(1),
        "missing bracket in region label must fail; got: {}",
        out
    );
    assert!(
        out.contains("]") || out.contains("region"),
        "expected diagnostic about region; got: {}",
        out
    );
}

#[test]
fn ref_type_missing_region_name() {
    let (out, code) = check_src("fn foo(&[] int x) { return 0; }\nfoo(nil);\n");
    assert_eq!(code, Some(1), "missing region name must fail; got: {}", out);
    assert!(
        out.contains("region") || out.contains("identifier"),
        "expected diagnostic about region name; got: {}",
        out
    );
}

// ========== Array Type Errors ==========

#[test]
fn array_type_missing_semicolon_before_size() {
    let (out, code) = check_src("fn foo([int 5] x) { return 0; }\nfoo(nil);\n");
    assert_eq!(
        code,
        Some(1),
        "missing semicolon in array type must fail; got: {}",
        out
    );
    assert!(
        out.contains(";") || out.contains("array"),
        "expected diagnostic about semicolon; got: {}",
        out
    );
}

#[test]
fn array_type_invalid_size() {
    let (out, code) = check_src("fn foo([int; -5] x) { return 0; }\nfoo(nil);\n");
    assert_eq!(code, Some(1), "negative array size must fail; got: {}", out);
    assert!(
        out.contains("non-negative") || out.contains("integer"),
        "expected diagnostic about size; got: {}",
        out
    );
}

// ========== Function Type Errors ==========

#[test]
fn fn_type_missing_closing_paren() {
    let (out, code) = check_src("fn foo(fn(int, int -> int x) { return x; }\nfoo(nil);\n");
    assert_eq!(
        code,
        Some(1),
        "missing closing paren in fn type must fail; got: {}",
        out
    );
    assert!(
        out.contains(")") || out.contains("fn(...)"),
        "expected diagnostic about paren; got: {}",
        out
    );
}

#[test]
fn fn_type_missing_arrow() {
    let (out, code) = check_src("fn foo(fn(int, int int x) { return x; }\nfoo(nil);\n");
    assert_eq!(
        code,
        Some(1),
        "missing arrow in fn type must fail; got: {}",
        out
    );
    assert!(
        out.contains(",") || out.contains(")"),
        "expected diagnostic about separator; got: {}",
        out
    );
}

// ========== Struct Name Errors ==========

#[test]
fn struct_decl_missing_identifier() {
    let (out, code) = check_src("struct 123 { int x }\nprint(0);\n");
    assert_eq!(
        code,
        Some(1),
        "numeric literal as struct name must fail; got: {}",
        out
    );
    assert!(
        out.contains("identifier") || out.contains("struct"),
        "expected diagnostic about identifier; got: {}",
        out
    );
}

#[test]
fn new_struct_missing_name() {
    let (out, code) = check_src("fn f() { let p = new 123 { x: 1 }; return 0; }\nf();\n");
    assert_eq!(code, Some(1), "numeric in new must fail; got: {}", out);
    assert!(
        out.contains("struct") || out.contains("name"),
        "expected diagnostic about struct name; got: {}",
        out
    );
}
