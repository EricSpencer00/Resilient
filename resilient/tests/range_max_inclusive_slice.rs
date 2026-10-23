//! RES-4788: inclusive slice endpoints at i64::MAX must saturate, not overflow.

fn run_ok(src: &str) -> String {
    let result = resilient::run_program(src);
    assert!(result.ok, "execution failed: {:?}", result.errors);
    result.stdout.trim().to_string()
}

#[test]
fn array_inclusive_slice_to_i64_max_clamps_to_length() {
    assert_eq!(
        run_ok("let a = [1, 2, 3]; println(a[0..=9223372036854775807]);"),
        "[1, 2, 3]"
    );
}

#[test]
fn string_inclusive_slice_to_i64_max_clamps_to_length() {
    assert_eq!(
        run_ok(r#"let s = "abc"; println(s[1..=9223372036854775807]);"#),
        "bc"
    );
}

#[test]
fn array_slice_builtin_inclusive_i64_max_clamps_to_length() {
    assert_eq!(
        run_ok("println(array_slice([1, 2, 3], 1, 9223372036854775807, true));"),
        "[2, 3]"
    );
}

#[test]
fn string_slice_builtin_inclusive_i64_max_clamps_to_length() {
    assert_eq!(
        run_ok(r#"println(array_slice("abc", 0, 9223372036854775807, true));"#),
        "abc"
    );
}

#[test]
fn ordinary_inclusive_slice_is_unchanged() {
    assert_eq!(run_ok("let a = [1, 2, 3, 4]; println(a[1..=2]);"), "[2, 3]");
}

#[test]
fn half_open_slice_at_i64_max_is_unchanged() {
    assert_eq!(
        run_ok("let a = [1, 2, 3]; println(a[0..9223372036854775807]);"),
        "[1, 2, 3]"
    );
}

#[test]
fn for_inclusive_range_ending_at_i64_max_yields_only_max() {
    assert_eq!(
        run_ok("for i in 9223372036854775806..=9223372036854775807 { println(i); }"),
        "9223372036854775806\n9223372036854775807"
    );
}
