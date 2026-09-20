//! RES-4702: std::iter::range must fail closed when its signed cursor overflows.

fn run_err(source: &str) -> String {
    let result = resilient::run_program(source);
    assert!(
        !result.ok,
        "expected range failure, got: {:?}",
        result.stdout
    );
    result.errors.join("\n")
}

#[test]
fn positive_cursor_overflow_is_typed_error() {
    let errors = run_err(
        r#"
        use std::iter;
        fn main() {
            iter::range(9223372036854775806, 9223372036854775807, 2);
        }
        main();
        "#,
    );
    assert!(
        errors.contains("iter::range: step overflow at signed boundary"),
        "unexpected error: {errors}"
    );
}

#[test]
fn negative_cursor_overflow_is_typed_error() {
    let errors = run_err(
        r#"
        use std::iter;
        fn main() {
            iter::range(-9223372036854775807, -9223372036854775808, -2);
        }
        main();
        "#,
    );
    assert!(
        errors.contains("iter::range: step overflow at signed boundary"),
        "unexpected error: {errors}"
    );
}

#[test]
fn ordinary_ranges_preserve_results() {
    let result = resilient::run_program(
        r#"
        use std::iter;
        fn main() {
            let ascending = iter::range(2, 6, 2);
            let descending = iter::range(6, 1, -2);
            println(len(ascending));
            println(ascending[1]);
            println(len(descending));
            println(descending[1]);
        }
        main();
        "#,
    );
    assert!(result.ok, "ordinary range failed: {:?}", result.errors);
    assert_eq!(result.stdout.trim(), "2\n4\n3\n4");
}
