//! RES-XXXX: Negative typecheck diagnostics tests.
//!
//! Tests that the typechecker rejects semantic errors with clear,
//! specific diagnostic messages. Each test runs `rz check` on a
//! small scratch file and asserts:
//! 1. Exit code is 1 (typecheck failure).
//! 2. Stderr or stdout contains the expected diagnostic substring.

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
        std::env::temp_dir().join(format!("res_negtc_{}_{}.rz", std::process::id(), n));
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

// --- Binary operator type mismatches ---

#[test]
fn binop_string_plus_array() {
    let src = "fn main(int _d) { return \"hi\" + [1]; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1), "expected typecheck failure; got:\n{out}");
    assert!(
        out.contains("cannot concatenate string"),
        "expected concatenate diagnostic; got:\n{out}"
    );
}

#[test]
fn binop_int_plus_bool() {
    let src = "fn main(int _d) { return 1 + true; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Cannot apply") || out.contains("type"),
        "expected type error; got:\n{out}"
    );
}

#[test]
fn binop_float_divide_bool() {
    let src = "fn main(int _d) { return 3.14 / true; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Cannot apply") || out.contains("type"),
        "expected type mismatch; got:\n{out}"
    );
}

#[test]
fn binop_bool_minus_int() {
    let src = "fn main(int _d) { return true - 5; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Cannot") || out.contains("type"),
        "expected error; got:\n{out}"
    );
}

#[test]
fn binop_array_divide_int() {
    let src = "fn main(int _d) { let xs = [1, 2]; return xs / 2; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Cannot apply") || out.contains("type"),
        "expected type error; got:\n{out}"
    );
}

#[test]
fn bitwise_and_bool_int() {
    let src = "fn main(int _d) { return true & 5; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Bitwise") || out.contains("requires int"),
        "expected bitwise error; got:\n{out}"
    );
}

#[test]
fn logical_or_int_bool() {
    let src = "fn main(int _d) { return 42 || true; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Logical") || out.contains("requires bool"),
        "expected logical error; got:\n{out}"
    );
}

#[test]
fn compare_array_int() {
    let src = "fn main(int _d) { return [1, 2] < 5; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Cannot compare"),
        "expected comparison error; got:\n{out}"
    );
}

#[test]
fn coalesce_non_option() {
    let src = "fn main(int _d) { return 5 ?? 10; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("requires an Option"),
        "expected option error; got:\n{out}"
    );
}

// --- Unary operator type mismatches ---

#[test]
fn unary_not_on_int() {
    let src = "fn main(int _d) { return !5; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Cannot apply '!'"),
        "expected unary ! error; got:\n{out}"
    );
}

#[test]
fn unary_minus_on_bool() {
    let src = "fn main(int _d) { return -true; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Cannot apply '-'"),
        "expected unary - error; got:\n{out}"
    );
}

#[test]
fn unary_minus_on_string() {
    let src = "fn main(int _d) { return -\"hi\"; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Cannot apply '-'"),
        "expected unary - error; got:\n{out}"
    );
}

// --- If/while condition type errors ---

#[test]
fn if_condition_int() {
    let src = "fn main(int _d) { if 42 { return 1; } return 0; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("condition") || out.contains("Bool"),
        "expected condition error; got:\n{out}"
    );
}

#[test]
fn while_condition_string() {
    let src = "fn main(int _d) { while \"loop\" { return 0; } return 0; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("condition") || out.contains("Bool"),
        "expected condition error; got:\n{out}"
    );
}

// --- Range bound type errors ---

#[test]
fn range_lower_bound_string() {
    let src = "fn main(int _d) { for i in \"lo\"..10 { return i; } return 0; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("range lower bound"),
        "expected range error; got:\n{out}"
    );
}

#[test]
fn range_upper_bound_bool() {
    let src = "fn main(int _d) { for i in 0..true { return i; } return 0; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("range upper bound"),
        "expected range error; got:\n{out}"
    );
}

// --- Assert/Assume type errors ---

#[test]
fn assert_condition_int() {
    let src = "fn main(int _d) { assert(42); return 0; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Assert condition"),
        "expected assert error; got:\n{out}"
    );
}

#[test]
fn assert_message_int() {
    let src = "fn main(int _d) { assert(true, 99); return 0; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Assert message") || out.contains("string"),
        "expected message error; got:\n{out}"
    );
}

#[test]
fn assume_condition_array() {
    let src = "fn main(int _d) { assume([1]); return 0; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Assume condition"),
        "expected assume error; got:\n{out}"
    );
}

#[test]
fn return_type_mismatch() {
    let src =
        "fn get_int() -> int { return true; } fn main(int _d) { return get_int(); } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("type") || out.contains("expected") || out.contains("return"),
        "expected return type error; got:\n{out}"
    );
}

// --- Function call errors ---

#[test]
fn call_undefined_function() {
    let src = "fn main(int _d) { return frob(1); } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("Undefined") || out.contains("not found") || out.contains("variable"),
        "expected undefined error; got:\n{out}"
    );
}

#[test]
fn call_wrong_arg_count() {
    let src =
        "fn add(int a, int b) { return a + b; } fn main(int _d) { return add(1); } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("arg") || out.contains("parameter"),
        "expected arity error; got:\n{out}"
    );
}

#[test]
fn call_wrong_arg_type() {
    let src = "fn takes_bool(bool b) { if b { return 1; } return 0; } fn main(int _d) { return takes_bool(42); } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("type") || out.contains("expected"),
        "expected arg type error; got:\n{out}"
    );
}

// --- Struct field access errors ---

#[test]
fn enum_constructor_arity_mismatch() {
    let src = "enum E { Some(int), None } fn main(int _d) { return E::Some(1, 2); } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("arg") || out.contains("expected"),
        "expected arg count error; got:\n{out}"
    );
}

#[test]
fn field_access_nonexistent_field() {
    let src =
        "struct P { int x } fn main(int _d) { let p = new P { x: 1 }; return p.y; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("has no field"),
        "expected field error; got:\n{out}"
    );
}

#[test]
fn array_subscript_out_of_bounds_type() {
    let src = "fn main(int _d) { let xs = [1, 2]; return xs[true]; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("bool") || out.contains("int"),
        "expected type error; got:\n{out}"
    );
}

// --- Type annotation assignment errors ---

#[test]
fn let_annotated_int_assigned_string() {
    let src = "fn main(int _d) { let x: int = \"hi\"; return x; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("type") || out.contains("expected"),
        "expected type mismatch; got:\n{out}"
    );
}

#[test]
fn let_annotated_bool_assigned_int() {
    let src = "fn main(int _d) { let b: bool = 42; return if b { 1 } else { 0 }; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("type") || out.contains("expected"),
        "expected type mismatch; got:\n{out}"
    );
}

// --- Match arm type inconsistency ---

#[test]
fn match_arm_type_mismatch() {
    let src = "fn main(int _d) { return match 5 { 1 => true, _ => 42 }; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("type") || out.contains("bool") || out.contains("int"),
        "expected type mismatch; got:\n{out}"
    );
}

// --- Array element type inconsistency ---

#[test]
fn array_mixed_types() {
    let src = "fn main(int _d) { return [1, \"hi\", 3]; } main(0);\n";
    let (out, code) = check_src(src);
    assert_eq!(code, Some(1));
    assert!(
        out.contains("element") || out.contains("type") || out.contains("array"),
        "expected array type error; got:\n{out}"
    );
}
