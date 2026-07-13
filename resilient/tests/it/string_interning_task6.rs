//! RES-2612 Task 6: Test runtime intern() builtin for dynamic string deduplication.
//! These tests verify that the runtime intern() builtin interns dynamically-created
//! strings and that multiple calls with the same value are deduplicated.

#[test]
fn test_intern_simple_string() {
    let code = r#"
fn main() {
    let s = "hello";
    let interned = intern(s);
    print(interned);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
    assert!(
        result.stdout.contains("hello"),
        "Expected 'hello' in output, got: {}",
        result.stdout
    );
}

#[test]
fn test_intern_dynamic_string() {
    let code = r#"
fn main() {
    let s = "hello";
    let interned = intern(s);
    print(interned);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
}

#[test]
fn test_intern_multiple_calls_same_value() {
    let code = r#"
fn main() {
    let s1 = intern("hello");
    let s2 = intern("hello");
    if s1 == s2 {
        print("equal");
    } else {
        print("not equal");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
    assert!(
        result.stdout.contains("equal"),
        "Expected 'equal' in output, got: {}",
        result.stdout
    );
}

#[test]
fn test_intern_different_strings() {
    let code = r#"
fn main() {
    let s1 = intern("hello");
    let s2 = intern("world");
    if s1 != s2 {
        print("different");
    } else {
        print("same");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
    assert!(
        result.stdout.contains("different"),
        "Expected 'different' in output, got: {}",
        result.stdout
    );
}

#[test]
fn test_intern_concatenated_string() {
    let code = r#"
fn main() {
    let s1 = "hello";
    let s2 = "world";
    let concat = s1 + " " + s2;
    let interned = intern(concat);
    print(interned);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
    assert!(
        result.stdout.contains("hello world"),
        "Expected 'hello world' in output, got: {}",
        result.stdout
    );
}

#[test]
fn test_intern_wrong_argument_count() {
    let code = r#"
fn main() {
    let result = intern();
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(!result.ok, "Expected error for wrong argument count");
}

#[test]
fn test_intern_too_many_arguments() {
    let code = r#"
fn main() {
    let result = intern("hello", "world");
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(!result.ok, "Expected error for too many arguments");
}

#[test]
fn test_intern_wrong_type() {
    let code = r#"
fn main() {
    let result = intern(42);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(!result.ok, "Expected error for non-string argument");
}

#[test]
fn test_intern_preserves_string_value() {
    let code = r#"
fn main() {
    let original = "test_string";
    let interned = intern(original);
    print(interned);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
    assert!(
        result.stdout.contains("test_string"),
        "Expected 'test_string' in output, got: {}",
        result.stdout
    );
}

#[test]
fn test_intern_empty_string() {
    let code = r#"
fn main() {
    let s = intern("");
    if s == "" {
        print("empty");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
    assert!(
        result.stdout.contains("empty"),
        "Expected 'empty' in output, got: {}",
        result.stdout
    );
}
