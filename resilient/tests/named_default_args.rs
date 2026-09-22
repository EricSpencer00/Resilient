//! Regression coverage for named calls using trailing default parameters.

#[test]
fn named_call_uses_trailing_default() {
    let result = resilient::run_program(
        r#"
fn add(int x, int y = 9) -> int { return x + y; }
fn main(int _d) { println(add(x: 3)); }
main(0);
"#,
    );

    assert!(
        result.ok,
        "named call should compile and run: {:?}",
        result.errors
    );
    assert_eq!(result.stdout.trim(), "12");
}

#[test]
fn named_call_reorders_and_fills_default() {
    let result = resilient::run_program(
        r#"
fn encode(int x, int y = 2, int z = 3) -> int {
    return x * 100 + y * 10 + z;
}
fn main(int _d) { println(encode(z: 3, x: 1)); }
main(0);
"#,
    );

    assert!(
        result.ok,
        "named call should compile and run: {:?}",
        result.errors
    );
    assert_eq!(result.stdout.trim(), "123");
}

#[test]
fn named_call_still_rejects_missing_required_parameter() {
    let result = resilient::run_program(
        r#"
fn add(int x, int y = 9) -> int { return x + y; }
fn main(int _d) { add(y: 2); }
main(0);
"#,
    );

    assert!(
        !result.ok,
        "missing required parameter must remain an error"
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("Missing argument for parameter `x`")),
        "unexpected diagnostics: {:?}",
        result.errors
    );
}
