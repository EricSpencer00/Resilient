/// RES-3209: Malformed-input regression corpus for macros validation.
///
/// Integration tests for macros validator covering malformed input cases.
/// Tests that the macros validator correctly rejects invalid declarations and
/// detects duplicate/conflicting macro registrations via the feature_attrs system.

use resilient::Node;

fn check_macros(decls: &[(&str, usize, &str)]) -> Result<(), String> {
    let _g = resilient::feature_attrs::lock_for_test();
    resilient::feature_attrs::reset();

    for (item_name, line, args) in decls {
        resilient::feature_attrs::record(
            item_name,
            resilient::feature_attrs::AttrRecord {
                name: "macro".into(),
                args: (*args).into(),
                line: *line,
            },
        );
    }

    let program = Node::Program(vec![]);
    let result = resilient::macros::check(&program, "test.res");

    resilient::feature_attrs::reset();
    result
}

#[test]
fn macros_malformed_missing_pattern_field() {
    let result = check_macros(&[("test_macro", 0, r#"expansion = "1 + 2""#)]);
    assert!(result.is_err(), "Should reject macro missing pattern field");
    let err = result.unwrap_err();
    assert!(err.contains("missing required `pattern` field"));
}

#[test]
fn macros_malformed_missing_expansion_field() {
    let result = check_macros(&[("test_macro", 0, r#"pattern = "$1 + $2""#)]);
    assert!(result.is_err(), "Should reject macro missing expansion field");
    let err = result.unwrap_err();
    assert!(err.contains("missing required `expansion` field"));
}

#[test]
fn macros_malformed_duplicate_pattern_field() {
    let result =
        check_macros(&[("test_macro", 0, r#"pattern = "$1 + $2", pattern = "$1 - $2", expansion = "$1""#)]);
    assert!(result.is_err(), "Should reject macro with duplicate pattern field");
    let err = result.unwrap_err();
    assert!(err.contains("duplicate `pattern` field"));
}

#[test]
fn macros_malformed_duplicate_expansion_field() {
    let result = check_macros(&[(
        "test_macro",
        0,
        r#"pattern = "$1 + $2", expansion = "$1", expansion = "$2""#,
    )]);
    assert!(result.is_err(), "Should reject macro with duplicate expansion field");
    let err = result.unwrap_err();
    assert!(err.contains("duplicate `expansion` field"));
}

#[test]
fn macros_malformed_unknown_field() {
    let result = check_macros(&[(
        "test_macro",
        0,
        r#"pattern = "$1", expansion = "$1", unknown = "value""#,
    )]);
    assert!(result.is_err(), "Should reject macro with unknown field");
    let err = result.unwrap_err();
    assert!(err.contains("unknown field"));
}

#[test]
fn macros_malformed_trailing_comma() {
    let result = check_macros(&[("test_macro", 0, r#"pattern = "$1", expansion = "$1",]"#)]);
    assert!(result.is_err(), "Should reject macro with trailing comma");
    let err = result.unwrap_err();
    assert!(err.contains("trailing comma"));
}

#[test]
fn macros_malformed_unterminated_string() {
    let result = check_macros(&[(
        "test_macro",
        0,
        r#"pattern = "$1 + $2, expansion = "$1 + $2""#,
    )]);
    assert!(result.is_err(), "Should reject macro with unterminated string");
    let err = result.unwrap_err();
    assert!(err.contains("unterminated quoted string"));
}

#[test]
fn macros_malformed_placeholder_zero() {
    let result = check_macros(&[("test_macro", 0, r#"pattern = "$0", expansion = "$0""#)]);
    assert!(result.is_err(), "Should reject macro with $0 placeholder");
    let err = result.unwrap_err();
    assert!(err.contains("placeholder indices start at 1"));
}

#[test]
fn macros_malformed_placeholder_leading_zero() {
    let result = check_macros(&[("test_macro", 0, r#"pattern = "$01", expansion = "$01""#)]);
    assert!(result.is_err(), "Should reject macro with leading zero in placeholder");
    let err = result.unwrap_err();
    assert!(err.contains("leading zeroes are not allowed"));
}

#[test]
fn macros_malformed_placeholder_multidigit() {
    let result = check_macros(&[("test_macro", 0, r#"pattern = "$12", expansion = "$1""#)]);
    assert!(result.is_err(), "Should reject macro with multi-digit placeholder");
    let err = result.unwrap_err();
    assert!(err.contains("multi-digit placeholders are not supported"));
}

#[test]
fn macros_malformed_placeholder_invalid_syntax() {
    let result = check_macros(&[("test_macro", 0, r#"pattern = "$", expansion = "$1""#)]);
    assert!(result.is_err(), "Should reject macro with invalid placeholder");
    let err = result.unwrap_err();
    assert!(err.contains("invalid placeholder"));
}

#[test]
fn macros_malformed_expansion_arity_mismatch() {
    let result =
        check_macros(&[("test_macro", 0, r#"pattern = "$1", expansion = "$1 + $2""#)]);
    assert!(result.is_err(), "Should reject expansion referencing undefined placeholder");
    let err = result.unwrap_err();
    assert!(err.contains("references placeholder"));
}

#[test]
fn macros_malformed_expansion_invalid_expression() {
    let result = check_macros(&[(
        "test_macro",
        0,
        r#"pattern = "$1", expansion = "1 2 3 + +""#,
    )]);
    assert!(result.is_err(), "Should reject expansion that doesn't parse");
    let err = result.unwrap_err();
    assert!(err.contains("does not parse after placeholder substitution"));
}

#[test]
fn macros_malformed_duplicate_macro_declaration() {
    let result = check_macros(&[
        ("same", 0, r#"pattern = "x", expansion = "42""#),
        ("same", 5, r#"pattern = "x", expansion = "42""#),
    ]);
    assert!(result.is_err(), "Should reject duplicate macro declaration");
    let err = result.unwrap_err();
    assert!(err.contains("duplicate") || err.contains("conflicting"));
}

#[test]
fn macros_malformed_conflicting_macro_declaration() {
    let result = check_macros(&[
        ("same", 0, r#"pattern = "x", expansion = "1""#),
        ("same", 5, r#"pattern = "x", expansion = "2""#),
    ]);
    assert!(result.is_err(), "Should reject conflicting macro declaration");
    let err = result.unwrap_err();
    assert!(err.contains("conflicting"));
}

#[test]
fn macros_valid_simple_macro() {
    let result = check_macros(&[("add", 0, r#"pattern = "$1 + $2", expansion = "$1 + $2""#)]);
    assert!(result.is_ok(), "Should accept valid macro");
}

#[test]
fn macros_valid_macro_no_args() {
    let result = check_macros(&[("pi", 0, r#"pattern = "PI", expansion = "3.14159""#)]);
    assert!(result.is_ok(), "Should accept macro with no placeholders");
}

#[test]
fn macros_valid_multiple_different_macros() {
    let result = check_macros(&[
        ("add", 0, r#"pattern = "$1 + $2", expansion = "$1 + $2""#),
        ("sub", 5, r#"pattern = "$1 - $2", expansion = "$1 - $2""#),
    ]);
    assert!(result.is_ok(), "Should accept multiple distinct macros");
}

#[test]
fn macros_valid_macro_with_escaped_quotes() {
    let result = check_macros(&[("greeting", 0, r#"pattern = "name", expansion = "\"Hello\"""#)]);
    assert!(result.is_ok(), "Should accept macro with escaped quotes");
}

#[test]
fn macros_valid_macro_complex_expression() {
    let result = check_macros(&[("or_op", 0, r#"pattern = "$1 or $2", expansion = "($1) || ($2)""#)]);
    assert!(result.is_ok(), "Should accept macro with complex expression");
}
