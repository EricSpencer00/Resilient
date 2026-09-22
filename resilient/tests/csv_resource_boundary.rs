//! RES-4716: CSV and TSV parsing must reject oversized materialization.

fn escaped_string_literal(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

#[test]
fn csv_parse_rejects_too_many_rows_before_materializing() {
    let input = "row\n".repeat(100_001);
    let code = format!(
        "let rows = csv_parse(\"{}\");",
        escaped_string_literal(&input)
    );
    let result = resilient::run_program(&code);

    assert!(!result.ok, "oversized CSV row count must fail closed");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("would exceed 100000 rows")),
        "unexpected diagnostics: {:?}",
        result.errors
    );
}

#[test]
fn csv_parse_rejects_too_many_fields_before_materializing() {
    let input = "field,".repeat(1_000_000);
    let code = format!(
        "let fields = csv_parse(\"{}\");",
        escaped_string_literal(&input)
    );
    let result = resilient::run_program(&code);

    assert!(!result.ok, "oversized CSV field count must fail closed");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("would exceed 1000000 fields")),
        "unexpected diagnostics: {:?}",
        result.errors
    );
}
