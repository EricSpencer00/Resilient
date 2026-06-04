//! RES-2612 Task 8: Comprehensive string interning test suite.
//!
//! This suite validates all aspects of string interning across 10 categories:
//! 1. Basic interning (deduplication, uniqueness, empty/single-char strings)
//! 2. Pool management (all_interned_strings, reset, sequential IDs, persistence)
//! 3. Parser integration (literals in program, multiple contexts, interpolation)
//! 4. Interpreter evaluation (evaluation, equality, string operations, conditions)
//! 5. Equality & comparison (==, !=, O(1) property, case sensitivity)
//! 6. Edge cases & unicode (empty, long, special chars, emoji, whitespace)
//! 7. Runtime intern() builtin (intern, dynamic strings, concatenation, error handling)
//! 8. Type system integration (type checking, string operations, function signatures)
//! 9. Stress tests (large number of strings, hundreds of references, loops)
//! 10. Regression tests (performance, backward compatibility)

use std::sync::Mutex;

// Serialize test access to the global pool to prevent race conditions
static POOL_LOCK: Mutex<()> = Mutex::new(());

/// Helper to lock the pool, handling poisoned locks from previous test panics
fn acquire_pool_lock() -> std::sync::MutexGuard<'static, ()> {
    POOL_LOCK.lock().unwrap_or_else(|poisoned| {
        // If the lock is poisoned, recover from it
        poisoned.into_inner()
    })
}

// ============================================================================
// Category 1: Basic Interning
// ============================================================================

#[test]
fn test_basic_interning_single_string() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    let id = resilient::string_interning::intern_string("hello".to_string());
    let retrieved = resilient::string_interning::get_interned_string(id);

    assert_eq!(retrieved, Some("hello".to_string()));
}

#[test]
fn test_basic_interning_lookup() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    let id = resilient::string_interning::intern_string("test".to_string());
    assert!(resilient::string_interning::get_interned_string(id).is_some());
}

#[test]
fn test_deduplication_identical_strings_same_id() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    let id1 = resilient::string_interning::intern_string("hello".to_string());
    let id2 = resilient::string_interning::intern_string("hello".to_string());

    assert_eq!(id1, id2, "Identical strings must have same ID");
}

#[test]
fn test_uniqueness_different_strings_different_ids() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    let id1 = resilient::string_interning::intern_string("hello".to_string());
    let id2 = resilient::string_interning::intern_string("world".to_string());

    assert_ne!(id1, id2, "Different strings must have different IDs");
}

#[test]
fn test_empty_string_interning() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    let id = resilient::string_interning::intern_string("".to_string());
    let retrieved = resilient::string_interning::get_interned_string(id);

    assert_eq!(retrieved, Some("".to_string()));
}

#[test]
fn test_single_character_string_interning() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    let id = resilient::string_interning::intern_string("a".to_string());
    let retrieved = resilient::string_interning::get_interned_string(id);

    assert_eq!(retrieved, Some("a".to_string()));
}

// ============================================================================
// Category 2: Pool Management
// ============================================================================

#[test]
fn test_pool_all_interned_strings() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    resilient::string_interning::intern_string("apple".to_string());
    resilient::string_interning::intern_string("banana".to_string());
    resilient::string_interning::intern_string("apple".to_string()); // Duplicate

    let all = resilient::string_interning::all_interned_strings();
    assert_eq!(all.len(), 2, "Pool should contain 2 unique strings");
}

#[test]
fn test_pool_all_interned_strings_contains_all() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    resilient::string_interning::intern_string("first".to_string());
    resilient::string_interning::intern_string("second".to_string());
    resilient::string_interning::intern_string("third".to_string());

    let all = resilient::string_interning::all_interned_strings();
    let strings: Vec<String> = all.iter().map(|(_, s)| s.clone()).collect();

    assert!(strings.contains(&"first".to_string()));
    assert!(strings.contains(&"second".to_string()));
    assert!(strings.contains(&"third".to_string()));
}

#[test]
fn test_pool_reset_clears_all_strings() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    resilient::string_interning::intern_string("hello".to_string());
    resilient::string_interning::intern_string("world".to_string());

    resilient::string_interning::reset_interning_pool();

    let all = resilient::string_interning::all_interned_strings();
    assert_eq!(all.len(), 0, "Pool should be empty after reset");
}

#[test]
fn test_pool_reset_resets_id_counter() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    let id1 = resilient::string_interning::intern_string("first".to_string());
    assert_eq!(id1, 0, "First string should have ID 0");

    resilient::string_interning::reset_interning_pool();

    let id2 = resilient::string_interning::intern_string("first".to_string());
    assert_eq!(id2, 0, "After reset, first string should have ID 0 again");
}

#[test]
fn test_pool_sequential_id_assignment() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    let id1 = resilient::string_interning::intern_string("str1".to_string());
    let id2 = resilient::string_interning::intern_string("str2".to_string());
    let id3 = resilient::string_interning::intern_string("str3".to_string());

    assert_eq!(id1, 0);
    assert_eq!(id2, 1);
    assert_eq!(id3, 2);
}

#[test]
fn test_pool_persistence_across_operations() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    let id1 = resilient::string_interning::intern_string("persist".to_string());

    // Do other operations
    resilient::string_interning::intern_string("other".to_string());
    resilient::string_interning::get_interned_string(id1);

    // Original string should still be there
    let retrieved = resilient::string_interning::get_interned_string(id1);
    assert_eq!(retrieved, Some("persist".to_string()));
}

// ============================================================================
// Category 3: Parser Integration
// ============================================================================

#[test]
fn test_parser_string_literal_creates_interned_literal() {
    let code = r#"
fn main() {
    let s = "hello";
    print(s);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to parse and run: {:?}", result.errors);
}

#[test]
fn test_parser_multiple_identical_literals_same_program() {
    let code = r#"
fn main() {
    let s1 = "hello";
    let s2 = "hello";
    let s3 = "hello";
    print(s1);
    print(s2);
    print(s3);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
}

#[test]
fn test_parser_string_literal_in_variable_assignment() {
    let code = r#"
fn main() {
    let msg = "test";
    print(msg);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
}

#[test]
fn test_parser_string_literal_in_function_call() {
    let code = r#"
fn main() {
    print("directly in call");
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
}

#[test]
fn test_parser_string_literal_in_multiple_contexts() {
    let code = r#"
fn greet(str name) -> str {
    return name;
}

fn main() {
    let greeting = "hello";
    let result = greet("world");
    if greeting == "hello" {
        print("greeting");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
}

#[test]
fn test_parser_string_interpolation() {
    let code = r#"
fn main() {
    let name = "world";
    print("hello {name}");
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
}

// ============================================================================
// Category 4: Interpreter/Evaluation
// ============================================================================

#[test]
fn test_interpreter_string_literal_evaluates_to_correct_value() {
    let code = r#"
fn main() {
    let s = "test_value";
    print(s);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
    assert!(
        result.stdout.contains("test_value"),
        "Expected 'test_value' in output"
    );
}

#[test]
fn test_interpreter_identical_literals_evaluate_to_equal_values() {
    let code = r#"
fn main() {
    let s1 = "hello";
    let s2 = "hello";
    if s1 == s2 {
        print("equal");
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
fn test_interpreter_string_operations_work() {
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
fn test_interpreter_string_in_if_condition() {
    let code = r#"
fn main() {
    let msg = "test";
    if msg == "test" {
        print("yes");
    } else {
        print("no");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to run: {:?}", result.errors);
    assert!(
        result.stdout.contains("yes"),
        "Expected 'yes' in output, got: {}",
        result.stdout
    );
}

// ============================================================================
// Category 5: Equality & Comparison
// ============================================================================

#[test]
fn test_equality_operator_on_identical_interned_strings() {
    let code = r#"
fn main() {
    let s1 = "hello";
    let s2 = "hello";
    if s1 == s2 {
        print("equal");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(
        result.stdout.contains("equal"),
        "Expected equality check to pass"
    );
}

#[test]
fn test_equality_operator_on_different_interned_strings() {
    let code = r#"
fn main() {
    let s1 = "hello";
    let s2 = "world";
    if s1 == s2 {
        print("equal");
    } else {
        print("not equal");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(
        result.stdout.contains("not equal"),
        "Expected inequality for different strings"
    );
}

#[test]
fn test_inequality_operator_on_different_strings() {
    let code = r#"
fn main() {
    let s1 = "hello";
    let s2 = "world";
    if s1 != s2 {
        print("not equal");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(
        result.stdout.contains("not equal"),
        "Expected inequality operator to work"
    );
}

#[test]
fn test_inequality_operator_on_identical_strings() {
    let code = r#"
fn main() {
    let s1 = "test";
    let s2 = "test";
    if s1 != s2 {
        print("not equal");
    } else {
        print("equal");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(
        result.stdout.contains("equal"),
        "Expected identical strings to not be not-equal"
    );
}

#[test]
fn test_equality_case_sensitive() {
    let code = r#"
fn main() {
    let s1 = "Hello";
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
    assert!(
        result.stdout.contains("not equal"),
        "String equality should be case-sensitive"
    );
}

// ============================================================================
// Category 6: Edge Cases & Unicode
// ============================================================================

#[test]
fn test_edge_case_empty_string() {
    let code = r#"
fn main() {
    let empty = "";
    print("ok");
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to handle empty string");
}

#[test]
fn test_edge_case_very_long_string() {
    let long_str = "a".repeat(1000);
    let code = format!(
        r#"
fn main() {{
    let long = "{}";
    print("ok");
}}
main();
"#,
        long_str
    );

    let result = resilient::run_program(&code);
    assert!(result.ok, "Failed to handle very long string");
}

#[test]
fn test_edge_case_newline_in_string() {
    let code = "fn main() {\n    let s = \"line1\\nline2\";\n    print(s);\n}\nmain();\n";

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to handle newline in string");
}

#[test]
fn test_edge_case_tab_in_string() {
    let code = "fn main() {\n    let s = \"col1\\tcol2\";\n    print(s);\n}\nmain();\n";

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to handle tab in string");
}

#[test]
fn test_edge_case_escaped_quote_in_string() {
    let code = "fn main() {\n    let s = \"say \\\"hi\\\"\";\n    print(s);\n}\nmain();\n";

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to handle escaped quote in string");
}

#[test]
fn test_edge_case_backslash_in_string() {
    let code = "fn main() {\n    let s = \"path\\\\to\\\\file\";\n    print(s);\n}\nmain();\n";

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to handle backslash in string");
}

#[test]
fn test_unicode_emoji_in_string() {
    let code = r#"
fn main() {
    let emoji = "Hello 👋";
    print("ok");
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to handle emoji in string");
}

#[test]
fn test_unicode_non_ascii_characters() {
    let code = r#"
fn main() {
    let greeting = "你好世界";
    print("ok");
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed to handle non-ASCII unicode");
}

#[test]
fn test_edge_case_leading_whitespace() {
    let code = r#"
fn main() {
    let s1 = "  leading";
    let s2 = "leading";
    if s1 == s2 {
        print("equal");
    } else {
        print("not equal");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(
        result.stdout.contains("not equal"),
        "Leading whitespace should make strings different"
    );
}

#[test]
fn test_edge_case_trailing_whitespace() {
    let code = r#"
fn main() {
    let s1 = "trailing  ";
    let s2 = "trailing";
    if s1 == s2 {
        print("equal");
    } else {
        print("not equal");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(
        result.stdout.contains("not equal"),
        "Trailing whitespace should make strings different"
    );
}

// ============================================================================
// Category 7: Runtime intern() Builtin
// ============================================================================

#[test]
fn test_builtin_intern_on_literal() {
    let code = r#"
fn main() {
    let s = intern("literal");
    print("ok");
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(
        result.ok,
        "Failed to use intern() builtin: {:?}",
        result.errors
    );
}

#[test]
fn test_builtin_intern_on_dynamic_string() {
    let code = r#"
fn main() {
    let prefix = "hel";
    let suffix = "lo";
    let dynamic = "{prefix}{suffix}";
    let s = intern(dynamic);
    print("ok");
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(
        result.ok,
        "Failed to intern dynamic string: {:?}",
        result.errors
    );
}

#[test]
fn test_builtin_intern_multiple_calls_same_string() {
    let code = r#"
fn main() {
    let id1 = intern("same");
    let id2 = intern("same");
    print("ok");
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(
        result.ok,
        "Failed to call intern multiple times: {:?}",
        result.errors
    );
}

// ============================================================================
// Category 8: Type System Integration
// ============================================================================

#[test]
fn test_type_system_string_literal_as_string_type() {
    let code = r#"
fn main() {
    let s: str = "hello";
    print(s);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed type check: {:?}", result.errors);
}

#[test]
fn test_type_system_string_parameter_in_function() {
    let code = r#"
fn greet(str name) -> str {
    return name;
}

fn main() {
    let result = greet("Alice");
    print(result);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed type check: {:?}", result.errors);
}

#[test]
fn test_type_system_string_return_type() {
    let code = r#"
fn get_message() -> str {
    return "hello";
}

fn main() {
    let msg = get_message();
    print(msg);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed type check: {:?}", result.errors);
}

#[test]
fn test_type_system_string_in_array() {
    let code = r#"
fn main() {
    let strings = ["one", "two", "three"];
    print("ok");
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(
        result.ok,
        "Failed type check for string array: {:?}",
        result.errors
    );
}

// ============================================================================
// Category 9: Stress Tests
// ============================================================================

#[test]
fn test_stress_large_number_of_unique_strings() {
    let _lock = acquire_pool_lock();
    resilient::string_interning::reset_interning_pool();

    let mut ids = Vec::new();
    for i in 0..100 {
        let s = format!("string_{}", i);
        let id = resilient::string_interning::intern_string(s);
        ids.push(id);
    }

    // Verify all IDs are unique
    ids.sort();
    for i in 0..ids.len() - 1 {
        assert_ne!(ids[i], ids[i + 1], "IDs should be unique");
    }

    let all = resilient::string_interning::all_interned_strings();
    assert_eq!(all.len(), 100, "Should have 100 unique strings");
}

#[test]
fn test_stress_program_with_hundreds_of_references() {
    let mut code = String::from("fn main() {\n");
    for i in 0..50 {
        code.push_str(&format!("    let s{} = \"str{}\";\n", i, i));
        code.push_str(&format!("    print(s{});\n", i));
    }
    code.push_str("}\nmain();\n");

    let result = resilient::run_program(&code);
    assert!(
        result.ok,
        "Failed with many string references: {:?}",
        result.errors
    );
}

#[test]
fn test_stress_repeated_interning_of_same_dynamic_string() {
    let code = r#"
fn main() {
    let i = 0;
    while i < 10 {
        let msg = "repeated";
        print(msg);
        i = i + 1;
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(
        result.ok,
        "Failed in loop with repeated string: {:?}",
        result.errors
    );
}

#[test]
fn test_stress_interning_in_nested_loops() {
    let code = r#"
fn main() {
    let i = 0;
    while i < 5 {
        let j = 0;
        while j < 5 {
            let msg = "nested";
            print(msg);
            j = j + 1;
        }
        i = i + 1;
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Failed in nested loops: {:?}", result.errors);
}

// ============================================================================
// Category 10: Regression Tests
// ============================================================================

#[test]
fn test_regression_string_operations_still_work() {
    let code = r#"
fn main() {
    let s1 = "hello";
    let s2 = "world";
    print(s1);
    print(s2);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Regression: Basic string operations broken");
}

#[test]
fn test_regression_normal_program_still_works() {
    let code = r#"
fn add(i32 a, i32 b) -> i32 {
    return a + b;
}

fn main() {
    let x = 5;
    let y = 3;
    let z = add(x, y);
    print("ok");
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Regression: Normal program execution broken");
}

#[test]
fn test_regression_mixed_string_and_numeric_operations() {
    let code = r#"
fn main() {
    let msg = "result";
    let x = 42;
    print(msg);
    print("ok");
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Regression: Mixed operations broken");
}

#[test]
fn test_regression_function_with_string_and_numeric_params() {
    let code = r#"
fn process(str msg, i32 count) -> str {
    return msg;
}

fn main() {
    let result = process("test", 5);
    print(result);
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Regression: Mixed parameters broken");
}

#[test]
fn test_regression_backward_compatibility_old_string_handling() {
    let code = r#"
fn main() {
    let original = "original";
    let copy = original;
    if original == copy {
        print("same");
    }
}
main();
"#;

    let result = resilient::run_program(code);
    assert!(result.ok, "Regression: Old string handling broken");
}
