//! Resource-bound regressions for recursive JSON parsing and encoding.

fn run_program(source: &str) -> resilient::RunResult {
    resilient::run_program(source)
}

fn nested_empty_json(depth: usize) -> String {
    format!(
        "{}{}{}",
        "[".repeat(depth.saturating_sub(1)),
        "[]",
        "]".repeat(depth.saturating_sub(1))
    )
}

#[test]
fn json_parser_accepts_the_depth_boundary() {
    let json = nested_empty_json(256);
    let source = format!("let value = from_json(\"{json}\");\nprintln(type_of(value));");
    let result = run_program(&source);

    assert!(
        result.ok,
        "depth boundary should remain valid: {:?}",
        result.errors
    );
    assert_eq!(result.stdout.trim(), "array");
}

#[test]
fn json_parser_and_validation_fail_closed_past_the_depth_boundary() {
    let json = nested_empty_json(257);
    let source = format!(
        "let json = \"{json}\";\nprintln(is_err(json_decode(json)));\nprintln(json_valid(json));"
    );
    let result = run_program(&source);

    assert!(
        result.ok,
        "safe JSON APIs should not abort the program: {:?}",
        result.errors
    );
    assert_eq!(result.stdout.trim(), "true\nfalse");

    let source = format!("let value = from_json(\"{json}\");");
    let result = run_program(&source);
    assert!(!result.ok, "from_json should reject excessive nesting");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("maximum JSON nesting depth")),
        "missing depth diagnostic: {:?}",
        result.errors
    );
}

#[test]
fn json_encoders_share_the_depth_boundary() {
    let boundary = nested_empty_json(256);

    for encoder in ["to_json", "json_encode", "json_encode_pretty"] {
        let source = format!("let value = from_json(\"{boundary}\");\nprintln({encoder}(value));");
        let result = run_program(&source);
        assert!(
            result.ok,
            "{encoder} should accept the depth boundary: {:?}",
            result.errors
        );
    }
}
