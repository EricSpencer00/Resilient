//! RES-4788: inclusive ranges must preserve the i64::MAX endpoint.

#[test]
fn inclusive_range_emits_i64_max() {
    let result = resilient::run_program(
        "for i in 9223372036854775807..=9223372036854775807 { println(i); }",
    );

    assert!(result.ok, "range execution failed: {:?}", result.errors);
    assert_eq!(result.stdout.trim(), "9223372036854775807");
}

#[test]
fn half_open_range_at_i64_max_remains_empty() {
    let result =
        resilient::run_program("for i in 9223372036854775807..9223372036854775807 { println(i); }");

    assert!(result.ok, "range execution failed: {:?}", result.errors);
    assert!(result.stdout.trim().is_empty());
}
