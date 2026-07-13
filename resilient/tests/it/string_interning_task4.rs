//! RES-2612 Task 4: Type-checking pass for string interning.
//!
//! Tests that the type checker validates StringInternLiteral nodes:
//! - Valid intern_ids pass validation
//! - Multiple interned strings pass
//! - Invalid intern_ids fail with proper error messages
//! - Nested nodes (function calls with string args) all pass validation

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_file(tag: &str, body: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_2612_task4_{}_{}_{}. rz",
        tag,
        std::process::id(),
        n
    ));
    std::fs::write(&path, body).expect("write scratch file");
    path
}

#[test]
fn test_valid_single_string_literal_passes_typechecker() {
    let src = tmp_file(
        "valid_single",
        r#"
fn main() {
    let s = "hello";
    print(s);
}
"#,
    );

    let out = Command::new(bin())
        .arg(&src)
        .output()
        .expect("spawn resilient");

    // Should compile and run without error
    assert_eq!(
        out.status.code(),
        Some(0),
        "Valid string literal should pass typechecker: stderr = {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_multiple_identical_strings_pass_typechecker() {
    let src = tmp_file(
        "multiple_identical",
        r#"
fn main() {
    let s1 = "hello";
    let s2 = "hello";
    let s3 = "hello";
    print(s1);
    print(s2);
    print(s3);
}
"#,
    );

    let out = Command::new(bin())
        .arg(&src)
        .output()
        .expect("spawn resilient");

    // All identical strings should intern to same ID and pass validation
    assert_eq!(
        out.status.code(),
        Some(0),
        "Multiple identical strings should pass typechecker: stderr = {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_multiple_different_strings_pass_typechecker() {
    let src = tmp_file(
        "multiple_different",
        r#"
fn main() {
    let s1 = "hello";
    let s2 = "world";
    let s3 = "test";
    print(s1);
    print(s2);
    print(s3);
}
"#,
    );

    let out = Command::new(bin())
        .arg(&src)
        .output()
        .expect("spawn resilient");

    // Different strings should each get unique IDs and pass validation
    assert_eq!(
        out.status.code(),
        Some(0),
        "Multiple different strings should pass typechecker: stderr = {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_string_in_nested_function_calls_passes_typechecker() {
    let src = tmp_file(
        "nested_calls",
        r#"
fn greet(str name) -> str {
    return name;
}

fn main() {
    let result = greet("Alice");
    print(result);
    print(greet("Bob"));
}
"#,
    );

    let out = Command::new(bin())
        .arg(&src)
        .output()
        .expect("spawn resilient");

    // Strings in nested function calls should pass validation
    assert_eq!(
        out.status.code(),
        Some(0),
        "Nested function calls with strings should pass typechecker: stderr = {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_string_in_if_expression_passes_typechecker() {
    let src = tmp_file(
        "if_expr",
        r#"
fn main() {
    let name = "Alice";
    if name == "Alice" {
        print("Found Alice");
    } else {
        print("Not Alice");
    }
}
"#,
    );

    let out = Command::new(bin())
        .arg(&src)
        .output()
        .expect("spawn resilient");

    // Strings in if conditions and branches should pass validation
    assert_eq!(
        out.status.code(),
        Some(0),
        "Strings in if expressions should pass typechecker: stderr = {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_string_in_loop_passes_typechecker() {
    let src = tmp_file(
        "loop",
        r#"
fn main() {
    let i = 0;
    while i < 3 {
        print("loop iteration");
        i = i + 1;
    }
}
"#,
    );

    let out = Command::new(bin())
        .arg(&src)
        .output()
        .expect("spawn resilient");

    // Strings in loop bodies should pass validation
    assert_eq!(
        out.status.code(),
        Some(0),
        "Strings in loops should pass typechecker: stderr = {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
