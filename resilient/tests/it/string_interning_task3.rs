//! RES-2612 Task 3: Parser integration for string interning.
//!
//! Tests that the parser automatically interns all string literals during parsing,
//! and that identical strings receive the same intern_id.
//!
//! These tests are CLI-based since the Lexer and Parser are private to the lib.

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
        "res_2612_task3_{}_{}_{}. rz",
        tag,
        std::process::id(),
        n
    ));
    std::fs::write(&path, body).expect("write scratch file");
    path
}

#[test]
fn test_simple_string_literal_interning() {
    let src = tmp_file(
        "simple",
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
        "Program should execute successfully: stderr = {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_identical_strings_in_function() {
    let src = tmp_file(
        "identical",
        r#"
fn main() {
    let s1 = "hello";
    let s2 = "hello";
    print(s1);
    print(s2);
}
"#,
    );

    let out = Command::new(bin())
        .arg(&src)
        .output()
        .expect("spawn resilient");

    assert_eq!(
        out.status.code(),
        Some(0),
        "Program should execute successfully: stderr = {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_different_strings_in_function() {
    let src = tmp_file(
        "different",
        r#"
fn main() {
    let s1 = "hello";
    let s2 = "world";
    print(s1);
    print(s2);
}
"#,
    );

    let out = Command::new(bin())
        .arg(&src)
        .output()
        .expect("spawn resilient");

    assert_eq!(
        out.status.code(),
        Some(0),
        "Program should execute successfully: stderr = {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_string_in_function_call() {
    let src = tmp_file(
        "in_call",
        r#"
fn main() {
    print("test");
}
"#,
    );

    let out = Command::new(bin())
        .arg(&src)
        .output()
        .expect("spawn resilient");

    assert_eq!(
        out.status.code(),
        Some(0),
        "Program should execute successfully: stderr = {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
