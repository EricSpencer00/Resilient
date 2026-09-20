//! Resource-bound regressions for callback-produced string output.

fn run(src: &str) -> resilient::RunResult {
    resilient::run_program(src)
}

#[test]
fn string_map_chars_rejects_output_above_limit() {
    let result = run(r#"
fn main() {
    let mapped = string_map_chars("aa", fn(string c) -> string {
        return string_repeat(c, 5000001);
    });
    println(mapped);
}
main();
"#);

    assert!(
        !result.ok,
        "expected output limit error: {:?}",
        result.errors
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("string_map_chars: output length")),
        "missing output-bound diagnostic: {:?}",
        result.errors
    );
}

#[test]
fn string_map_chars_allows_exact_output_limit() {
    let result = run(r#"
fn main() {
    let mapped = string_map_chars("a", fn(string c) -> string {
        return string_repeat(c, 10000000);
    });
    println(len(mapped));
}
main();
"#);

    assert!(result.ok, "exact-limit output failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("10000000"),
        "unexpected output: {}",
        result.stdout
    );
}
