//! Regression coverage for RES-4581's float-to-int upper-bound check.

fn run(source: &str) -> resilient::RunResult {
    resilient::run_program(source)
}

#[test]
fn round_to_int_rejects_first_float_above_i64_max() {
    let result = run(r#"
let value = 9223372036854775808.0;
round_to_int(value);
"#);

    assert!(
        !result.ok,
        "out-of-range round should fail: {:?}",
        result.errors
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("out of i64 range")),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn trunc_to_int_rejects_first_float_above_i64_max() {
    let result = run(r#"
let value = 9223372036854775808.0;
trunc_to_int(value);
"#);

    assert!(
        !result.ok,
        "out-of-range truncation should fail: {:?}",
        result.errors
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("out of i64 range")),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn largest_representable_positive_float_integer_remains_valid() {
    let result = run(r#"
let value = 9223372036854774784.0;
print(to_string(trunc_to_int(value)));
"#);

    assert!(
        result.ok,
        "boundary-adjacent value should succeed: {:?}",
        result.errors
    );
    assert_eq!(result.stdout, "9223372036854774784");
}
