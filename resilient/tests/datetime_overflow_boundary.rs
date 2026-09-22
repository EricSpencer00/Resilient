//! Regression coverage for fail-closed DateTime Unix conversion.

#[test]
fn datetime_to_unix_rejects_extreme_year_without_panicking() {
    let result = resilient::run_program(
        r#"
struct DateTime {
    int year,
    int month,
    int day,
    int hour,
    int minute,
    int second,
    int nanos,
}

let value = new DateTime {
    year: -999999999,
    month: 1,
    day: 1,
    hour: 0,
    minute: 0,
    second: 0,
    nanos: 0,
};
datetime_to_unix(value);
"#,
    );

    assert!(!result.ok, "an unrepresentable DateTime must be rejected");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("datetime_to_unix") && error.contains("overflow")),
        "unexpected diagnostics: {:?}",
        result.errors
    );
}

#[test]
fn datetime_to_unix_rejects_invalid_components_without_panicking() {
    let result = resilient::run_program(
        r#"
struct DateTime {
    int year,
    int month,
    int day,
    int hour,
    int minute,
    int second,
    int nanos,
}

let value = new DateTime {
    year: 2024,
    month: 2,
    day: 29,
    hour: 24,
    minute: 0,
    second: 0,
    nanos: 0,
};
datetime_to_unix(value);
"#,
    );

    assert!(!result.ok, "an invalid time component must be rejected");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("datetime_to_unix") && error.contains("hour")),
        "unexpected diagnostics: {:?}",
        result.errors
    );
}

#[test]
fn datetime_from_unix_extreme_boundary_is_non_panicking() {
    let result = resilient::run_program(
        "let value = datetime_from_unix(9223372036854775807);\nprintln(value);\n",
    );

    assert!(
        result.ok
            || result
                .errors
                .iter()
                .all(|error| !error.contains("panicked")),
        "extreme Unix input must not panic: {:?}",
        result.errors
    );
}
