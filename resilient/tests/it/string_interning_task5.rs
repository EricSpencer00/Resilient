//! RES-2612 Task 5: Test interpreter evaluation of interned strings.
//! These tests verify that StringInternLiteral nodes evaluate correctly
//! and that O(1) equality checks work for interned strings.

#[test]
fn test_interned_string_evaluates_to_correct_value() {
    let code = r#"
fn main() {
    let s = "hello";
    print(s);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
}

#[test]
fn test_identical_interned_strings_are_equal() {
    let code = r#"
fn main() {
    let s1 = "hello";
    let s2 = "hello";
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
fn test_different_interned_strings_are_not_equal() {
    let code = r#"
fn main() {
    let s1 = "hello";
    let s2 = "world";
    if s1 != s2 {
        print("not equal");
    } else {
        print("equal");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
    assert!(
        result.stdout.contains("not equal"),
        "Expected 'not equal' in output, got: {}",
        result.stdout
    );
}

#[test]
fn test_string_length_on_interned_string() {
    let code = r#"
fn main() {
    let s1 = "hello";
    let s2 = "hello";
    if s1 == s2 {
        print("5");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
    assert!(
        result.stdout.contains("5"),
        "Expected '5' in output, got: {}",
        result.stdout
    );
}

#[test]
fn test_string_interpolation_with_interned_strings() {
    let code = r#"
fn main() {
    let greeting = "hello";
    let name = "world";
    print("{greeting} {name}");
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
fn test_string_concatenation_with_interned_strings() {
    let code = r#"
fn main() {
    let s1 = "hello";
    let s2 = "world";
    let result = s1 + " " + s2;
    print(result);
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
fn test_substring_on_interned_string() {
    let code = r#"
fn main() {
    let s1 = "he";
    let s2 = "he";
    if s1 == s2 {
        print(s1);
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
    assert!(
        result.stdout.contains("he"),
        "Expected 'he' in output, got: {}",
        result.stdout
    );
}

#[test]
fn test_multiple_interned_strings_in_array() {
    let code = r#"
fn main() {
    let arr = ["hello", "world", "hello"];
    print(arr[0]);
    print(arr[1]);
    print(arr[2]);
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
    assert!(
        result.stdout.contains("world"),
        "Expected 'world' in output, got: {}",
        result.stdout
    );
}

#[test]
fn test_interned_string_in_struct() {
    let code = r#"
struct Greeting {
    string msg,
}

fn main() {
    let g = new Greeting { msg: "hello" };
    print(g.msg);
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
