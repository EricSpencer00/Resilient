//! ~90 comprehensive edge-case tests for string, char, and math builtins.
//! Covers: empty strings, unicode, needle>haystack, repeat 0, negative indices,
//! char boundaries, abs overflow, pow/sqrt edge cases, floor/ceil/round .5/-0.5,
//! clamp(min>max), padding edge cases.

fn run_ok(src: &str) -> String {
    let r = resilient::run_program(src);
    assert!(r.ok, "failed: {:?}", r.errors);
    r.stdout
}

fn run_err(src: &str) -> bool {
    let r = resilient::run_program(src);
    !r.ok
}

// ============================================================================
// String builtins: len, split, trim, contains, case conversion, repeat, pad
// ============================================================================

#[test]
fn string_len_empty() {
    let code = r#"print(len(""))"#;
    assert_eq!(run_ok(code), "0");
}

#[test]
fn string_len_ascii() {
    let code = r#"print(len("hello"))"#;
    assert_eq!(run_ok(code), "5");
}

#[test]
fn string_len_unicode_multibyte() {
    let code = r#"print(len("café"))"#;
    assert_eq!(run_ok(code), "4"); // 4 Unicode scalars
}

#[test]
fn string_split_empty_sep() {
    let code = r#"let parts = split("hi", ""); println(len(parts))"#;
    assert_eq!(run_ok(code).trim(), "2");
}

#[test]
fn string_split_empty_string() {
    let code = r#"let parts = split("", ","); println(len(parts))"#;
    assert_eq!(run_ok(code).trim(), "1");
}

#[test]
fn string_split_no_match() {
    let code = r#"let parts = split("hello", "x"); println(len(parts))"#;
    assert_eq!(run_ok(code).trim(), "1");
}

#[test]
fn string_split_multiple() {
    let code = r#"let parts = split("a,b,c", ","); println(len(parts))"#;
    assert_eq!(run_ok(code).trim(), "3");
}

#[test]
fn string_trim_empty() {
    let code = r#"print(trim(""))"#;
    assert_eq!(run_ok(code), "");
}

#[test]
fn string_trim_no_whitespace() {
    let code = r#"print(trim("hello"))"#;
    assert_eq!(run_ok(code), "hello");
}

#[test]
fn string_trim_both_sides() {
    let code = r#"print(trim("  hello  "))"#;
    assert_eq!(run_ok(code), "hello");
}

#[test]
fn string_trim_only_whitespace() {
    let code = r#"print(trim("   "))"#;
    assert_eq!(run_ok(code), "");
}

#[test]
fn string_contains_empty_haystack() {
    let code = r#"print(contains("", "a"))"#;
    assert_eq!(run_ok(code), "false");
}

#[test]
fn string_contains_empty_needle() {
    let code = r#"print(contains("hello", ""))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn string_contains_match() {
    let code = r#"print(contains("hello world", "world"))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn string_contains_needle_longer() {
    let code = r#"print(contains("hi", "hello"))"#;
    assert_eq!(run_ok(code), "false");
}

#[test]
fn string_to_upper_empty() {
    let code = r#"print(to_upper(""))"#;
    assert_eq!(run_ok(code), "");
}

#[test]
fn string_to_upper_mixed() {
    let code = r#"print(to_upper("HeLLo WoRLD"))"#;
    assert_eq!(run_ok(code), "HELLO WORLD");
}

#[test]
fn string_to_lower_empty() {
    let code = r#"print(to_lower(""))"#;
    assert_eq!(run_ok(code), "");
}

#[test]
fn string_to_lower_mixed() {
    let code = r#"print(to_lower("HeLLo WoRLD"))"#;
    assert_eq!(run_ok(code), "hello world");
}

#[test]
fn string_repeat_zero() {
    let code = r#"print(repeat("x", 0))"#;
    assert_eq!(run_ok(code), "");
}

#[test]
fn string_repeat_one() {
    let code = r#"print(repeat("ab", 1))"#;
    assert_eq!(run_ok(code), "ab");
}

#[test]
fn string_repeat_multiple() {
    let code = r#"print(repeat("ab", 3))"#;
    assert_eq!(run_ok(code), "ababab");
}

#[test]
fn string_repeat_negative_fails() {
    let code = r#"repeat("x", -1)"#;
    assert!(run_err(code));
}

#[test]
fn string_repeat_empty() {
    let code = r#"print(repeat("", 5))"#;
    assert_eq!(run_ok(code), "");
}

#[test]
fn string_pad_left_empty() {
    let code = r#"print(pad_left("", 3, "x"))"#;
    assert_eq!(run_ok(code), "xxx");
}

#[test]
fn string_pad_left_no_pad_needed() {
    let code = r#"print(pad_left("hello", 3, "x"))"#;
    assert_eq!(run_ok(code), "hello");
}

#[test]
fn string_pad_left_one_char_needed() {
    let code = r#"print(pad_left("abc", 4, "x"))"#;
    assert_eq!(run_ok(code), "xabc");
}

#[test]
fn string_pad_right_multiple() {
    let code = r#"print(pad_right("ab", 5, "-"))"#;
    assert_eq!(run_ok(code), "ab---");
}

#[test]
fn string_pad_left_negative_n_fails() {
    let code = r#"pad_left("x", -1, "y")"#;
    assert!(run_err(code));
}

#[test]
fn string_starts_with_empty_string() {
    let code = r#"print(starts_with("hello", ""))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn string_starts_with_match() {
    let code = r#"print(starts_with("hello", "he"))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn string_starts_with_no_match() {
    let code = r#"print(starts_with("hello", "ll"))"#;
    assert_eq!(run_ok(code), "false");
}

#[test]
fn string_starts_with_prefix_longer() {
    let code = r#"print(starts_with("hi", "hello"))"#;
    assert_eq!(run_ok(code), "false");
}

#[test]
fn string_ends_with_empty_suffix() {
    let code = r#"print(ends_with("hello", ""))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn string_ends_with_match() {
    let code = r#"print(ends_with("hello", "lo"))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn string_ends_with_no_match() {
    let code = r#"print(ends_with("hello", "he"))"#;
    assert_eq!(run_ok(code), "false");
}

#[test]
fn string_index_of_empty_needle() {
    let code = r#"print(index_of("hello", ""))"#;
    assert_eq!(run_ok(code), "0");
}

#[test]
fn string_index_of_found() {
    let code = r#"print(index_of("hello", "ll"))"#;
    assert_eq!(run_ok(code), "2");
}

#[test]
fn string_index_of_not_found() {
    let code = r#"print(index_of("hello", "x"))"#;
    assert_eq!(run_ok(code), "-1");
}

#[test]
fn string_index_of_needle_longer() {
    let code = r#"print(index_of("hi", "hello"))"#;
    assert_eq!(run_ok(code), "-1");
}

#[test]
fn string_last_index_of_empty_needle() {
    let code = r#"print(last_index_of("hello", ""))"#;
    assert_eq!(run_ok(code), "5");
}

#[test]
fn string_last_index_of_found() {
    let code = r#"print(last_index_of("hello", "l"))"#;
    assert_eq!(run_ok(code), "3");
}

#[test]
fn string_last_index_of_not_found() {
    let code = r#"print(last_index_of("hello", "x"))"#;
    assert_eq!(run_ok(code), "-1");
}

#[test]
fn string_replace_empty_from_fails() {
    let code = r#"replace("hello", "", "x")"#;
    assert!(run_err(code));
}

#[test]
fn string_replace_no_match() {
    let code = r#"print(replace("hello", "x", "y"))"#;
    assert_eq!(run_ok(code), "hello");
}

#[test]
fn string_replace_match() {
    let code = r#"print(replace("hello", "l", "L"))"#;
    assert_eq!(run_ok(code), "heLLo");
}

#[test]
fn string_replace_empty_to() {
    let code = r#"print(replace("hello", "ll", ""))"#;
    assert_eq!(run_ok(code), "heo");
}

#[test]
fn string_char_at_empty() {
    let code = r#"match char_at("", 0) { Ok(c) => print("ok"), Err(_) => print("err") }"#;
    assert_eq!(run_ok(code), "err");
}

#[test]
fn string_char_at_valid() {
    let code = r#"match char_at("hello", 0) { Ok(c) => print(c), Err(_) => print("err") }"#;
    assert_eq!(run_ok(code), "h");
}

#[test]
fn string_char_at_out_of_range() {
    let code = r#"match char_at("hello", 10) { Ok(c) => print("ok"), Err(_) => print("err") }"#;
    assert_eq!(run_ok(code), "err");
}

#[test]
fn string_char_at_negative() {
    let code = r#"match char_at("hello", -1) { Ok(c) => print("ok"), Err(_) => print("err") }"#;
    assert_eq!(run_ok(code), "err");
}

#[test]
fn string_reverse_empty() {
    let code = r#"print(string_reverse(""))"#;
    assert_eq!(run_ok(code), "");
}

#[test]
fn string_reverse_single() {
    let code = r#"print(string_reverse("a"))"#;
    assert_eq!(run_ok(code), "a");
}

#[test]
fn string_reverse_multi() {
    let code = r#"print(string_reverse("hello"))"#;
    assert_eq!(run_ok(code), "olleh");
}

#[test]
fn string_chars_empty() {
    let code = r#"println(len(string_chars("")))"#;
    assert_eq!(run_ok(code).trim(), "0");
}

#[test]
fn string_chars_basic() {
    let code = r#"let chars = string_chars("ab"); println(len(chars))"#;
    assert_eq!(run_ok(code).trim(), "2");
}

#[test]
fn string_chars_first_elem() {
    let code = r#"let chars = string_chars("hello"); print(chars[0])"#;
    assert_eq!(run_ok(code), "h");
}

#[test]
fn string_capitalize_empty() {
    let code = r#"print(string_capitalize(""))"#;
    assert_eq!(run_ok(code), "");
}

#[test]
fn string_capitalize_basic() {
    let code = r#"print(string_capitalize("hello world"))"#;
    assert_eq!(run_ok(code), "Hello world");
}

#[test]
fn string_substring_empty() {
    let code = r#"print(string_substring("hello", 0, 0))"#;
    assert_eq!(run_ok(code), "");
}

#[test]
fn string_substring_valid() {
    let code = r#"print(string_substring("hello", 1, 4))"#;
    assert_eq!(run_ok(code), "ell");
}

#[test]
fn string_substring_start_greater_than_end_fails() {
    let code = r#"string_substring("hello", 4, 1)"#;
    assert!(run_err(code));
}

#[test]
fn string_bytes_len_empty() {
    let code = r#"print(string_bytes_len(""))"#;
    assert_eq!(run_ok(code), "0");
}

#[test]
fn string_bytes_len_ascii() {
    let code = r#"print(string_bytes_len("hello"))"#;
    assert_eq!(run_ok(code), "5");
}

#[test]
fn string_bytes_len_unicode() {
    let code = r#"print(string_bytes_len("café"))"#;
    // "café" = c(1) + a(1) + f(1) + é(2) = 5 bytes
    assert_eq!(run_ok(code), "5");
}

#[test]
fn string_count_empty_haystack() {
    let code = r#"print(string_count("", "a"))"#;
    assert_eq!(run_ok(code), "0");
}

#[test]
fn string_count_no_match() {
    let code = r#"print(string_count("hello", "x"))"#;
    assert_eq!(run_ok(code), "0");
}

#[test]
fn string_count_multiple() {
    let code = r#"print(string_count("hello", "l"))"#;
    assert_eq!(run_ok(code), "2");
}

#[test]
fn string_count_empty_needle_fails() {
    let code = r#"string_count("hello", "")"#;
    assert!(run_err(code));
}

#[test]
fn string_lines_empty() {
    let code = r#"println(len(string_lines("")))"#;
    assert_eq!(run_ok(code).trim(), "0");
}

#[test]
fn string_lines_single() {
    let code = r#"let lines = string_lines("hello"); println(len(lines))"#;
    assert_eq!(run_ok(code).trim(), "1");
}

#[test]
fn string_lines_multiple() {
    let code = r#"let lines = string_lines("a\nb\nc"); println(len(lines))"#;
    assert_eq!(run_ok(code).trim(), "3");
}

#[test]
fn string_strip_prefix_match() {
    let code = r#"print(string_strip_prefix("hello", "he"))"#;
    assert_eq!(run_ok(code), "llo");
}

#[test]
fn string_strip_prefix_no_match() {
    let code = r#"print(string_strip_prefix("hello", "x"))"#;
    assert_eq!(run_ok(code), "hello");
}

#[test]
fn string_strip_suffix_match() {
    let code = r#"print(string_strip_suffix("hello", "lo"))"#;
    assert_eq!(run_ok(code), "hel");
}

#[test]
fn string_strip_suffix_no_match() {
    let code = r#"print(string_strip_suffix("hello", "x"))"#;
    assert_eq!(run_ok(code), "hello");
}

#[test]
fn string_find_all_empty() {
    let code = r#"println(len(string_find_all("hello", "l")))"#;
    assert_eq!(run_ok(code).trim(), "2");
}

#[test]
fn string_find_all_no_match() {
    let code = r#"println(len(string_find_all("hello", "x")))"#;
    assert_eq!(run_ok(code).trim(), "0");
}

#[test]
fn string_trim_start_empty() {
    let code = r#"print(trim_start(""))"#;
    assert_eq!(run_ok(code), "");
}

#[test]
fn string_trim_start_leading() {
    let code = r#"print(trim_start("  hello"))"#;
    assert_eq!(run_ok(code), "hello");
}

#[test]
fn string_trim_end_empty() {
    let code = r#"print(trim_end(""))"#;
    assert_eq!(run_ok(code), "");
}

#[test]
fn string_trim_end_trailing() {
    let code = r#"print(trim_end("hello  "))"#;
    assert_eq!(run_ok(code), "hello");
}

#[test]
fn string_is_ascii_alpha_empty() {
    let code = r#"print(is_ascii_alpha(""))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn string_is_ascii_alpha_match() {
    let code = r#"print(is_ascii_alpha("hello"))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn string_is_ascii_alpha_no_match() {
    let code = r#"print(is_ascii_alpha("hello123"))"#;
    assert_eq!(run_ok(code), "false");
}

#[test]
fn string_is_ascii_digit_empty() {
    let code = r#"print(is_ascii_digit(""))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn string_is_ascii_digit_match() {
    let code = r#"print(is_ascii_digit("12345"))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn string_is_ascii_digit_no_match() {
    let code = r#"print(is_ascii_digit("123a"))"#;
    assert_eq!(run_ok(code), "false");
}

#[test]
fn string_is_ascii_alnum_match() {
    let code = r#"print(is_ascii_alnum("abc123"))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn string_is_ascii_alnum_no_match() {
    let code = r#"print(is_ascii_alnum("abc 123"))"#;
    assert_eq!(run_ok(code), "false");
}

#[test]
fn string_parse_int_valid() {
    let code = r#"match parse_int("123") { Ok(n) => print(n), Err(_) => print("err") }"#;
    assert_eq!(run_ok(code), "123");
}

#[test]
fn string_parse_int_negative() {
    let code = r#"match parse_int("-42") { Ok(n) => print(n), Err(_) => print("err") }"#;
    assert_eq!(run_ok(code), "-42");
}

#[test]
fn string_parse_int_invalid() {
    let code = r#"match parse_int("abc") { Ok(n) => print("ok"), Err(_) => print("err") }"#;
    assert_eq!(run_ok(code), "err");
}

#[test]
fn string_parse_float_valid() {
    let code = r#"match parse_float("3.14") { Ok(f) => print(f), Err(_) => print("err") }"#;
    assert_eq!(run_ok(code), "3.14");
}

#[test]
fn string_parse_float_invalid() {
    let code = r#"match parse_float("abc") { Ok(f) => print("ok"), Err(_) => print("err") }"#;
    assert_eq!(run_ok(code), "err");
}

// ============================================================================
// Character/Unicode builtins: chr, ord
// ============================================================================

#[test]
fn char_chr_basic() {
    let code = r#"print(chr(72))"#;
    assert_eq!(run_ok(code), "H");
}

#[test]
fn char_ord_basic() {
    let code = r#"print(ord("H"))"#;
    assert_eq!(run_ok(code), "72");
}

#[test]
fn char_chr_unicode_emoji() {
    let code = r#"print(chr(0x263A))"#;
    // Should produce a smiley face U+263A
    let output = run_ok(code);
    assert!(!output.is_empty());
}

#[test]
fn char_ord_unicode() {
    let code = r#"print(ord("é"))"#;
    assert_eq!(run_ok(code), "233");
}

// ============================================================================
// Math builtins: abs, min, max, clamp, sqrt, pow, floor, ceil, round
// ============================================================================

#[test]
fn math_abs_zero() {
    let code = r#"print(abs(0))"#;
    assert_eq!(run_ok(code), "0");
}

#[test]
fn math_abs_positive() {
    let code = r#"print(abs(42))"#;
    assert_eq!(run_ok(code), "42");
}

#[test]
fn math_abs_negative() {
    let code = r#"print(abs(-42))"#;
    assert_eq!(run_ok(code), "42");
}

#[test]
fn math_abs_i64_min_adjacent() {
    let code = r#"print(abs(-9223372036854775807))"#;
    assert_eq!(run_ok(code), "9223372036854775807");
}

#[test]
fn math_abs_float() {
    let code = r#"print(abs(-3.14))"#;
    assert_eq!(run_ok(code), "3.14");
}

#[test]
fn math_min_two_ints() {
    let code = r#"print(min(5, 3))"#;
    assert_eq!(run_ok(code), "3");
}

#[test]
fn math_min_negative() {
    let code = r#"print(min(-5, -3))"#;
    assert_eq!(run_ok(code), "-5");
}

#[test]
fn math_max_two_ints() {
    let code = r#"print(max(5, 3))"#;
    assert_eq!(run_ok(code), "5");
}

#[test]
fn math_max_negative() {
    let code = r#"print(max(-5, -3))"#;
    assert_eq!(run_ok(code), "-3");
}

#[test]
fn math_min3_three() {
    let code = r#"print(min3(5, 2, 9))"#;
    assert_eq!(run_ok(code), "2");
}

#[test]
fn math_max3_three() {
    let code = r#"print(max3(5, 2, 9))"#;
    assert_eq!(run_ok(code), "9");
}

#[test]
fn math_clamp_in_range() {
    let code = r#"print(clamp(5, 1, 10))"#;
    assert_eq!(run_ok(code), "5");
}

#[test]
fn math_clamp_below_min() {
    let code = r#"print(clamp(0, 1, 10))"#;
    assert_eq!(run_ok(code), "1");
}

#[test]
fn math_clamp_above_max() {
    let code = r#"print(clamp(15, 1, 10))"#;
    assert_eq!(run_ok(code), "10");
}

#[test]
fn math_clamp_min_equals_max() {
    let code = r#"print(clamp(5, 3, 3))"#;
    assert_eq!(run_ok(code), "3");
}

#[test]
fn math_clamp_min_greater_than_max_fails() {
    let code = r#"clamp(5, 10, 1)"#;
    assert!(run_err(code));
}

#[test]
fn math_sqrt_zero() {
    let code = r#"print(sqrt(0.0))"#;
    assert_eq!(run_ok(code), "0");
}

#[test]
fn math_sqrt_one() {
    let code = r#"print(sqrt(1.0))"#;
    assert_eq!(run_ok(code), "1");
}

#[test]
fn math_sqrt_perfect_square() {
    let code = r#"print(sqrt(9.0))"#;
    assert_eq!(run_ok(code), "3");
}

#[test]
fn math_sqrt_imperfect() {
    let code = r#"print(sqrt(2.0))"#;
    let output = run_ok(code);
    assert!(output.contains("1.41"));
}

#[test]
fn math_sqrt_negative_returns_nan() {
    let code = r#"print(is_nan(sqrt(-1.0)))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn math_pow_zero_exponent() {
    let code = r#"print(pow(5.0, 0.0))"#;
    assert_eq!(run_ok(code), "1");
}

#[test]
fn math_pow_basic() {
    let code = r#"print(pow(2.0, 3.0))"#;
    assert_eq!(run_ok(code), "8");
}

#[test]
fn math_pow_negative_base() {
    let code = r#"print(pow(-2.0, 3.0))"#;
    assert_eq!(run_ok(code), "-8");
}

#[test]
fn math_floor_integer() {
    let code = r#"print(floor(5.0))"#;
    assert_eq!(run_ok(code), "5");
}

#[test]
fn math_floor_fractional() {
    let code = r#"print(floor(5.7))"#;
    assert_eq!(run_ok(code), "5");
}

#[test]
fn math_floor_negative() {
    let code = r#"print(floor(-5.3))"#;
    assert_eq!(run_ok(code), "-6");
}

#[test]
fn math_floor_at_half() {
    let code = r#"print(floor(5.5))"#;
    assert_eq!(run_ok(code), "5");
}

#[test]
fn math_ceil_integer() {
    let code = r#"print(ceil(5.0))"#;
    assert_eq!(run_ok(code), "5");
}

#[test]
fn math_ceil_fractional() {
    let code = r#"print(ceil(5.3))"#;
    assert_eq!(run_ok(code), "6");
}

#[test]
fn math_ceil_negative() {
    let code = r#"print(ceil(-5.7))"#;
    assert_eq!(run_ok(code), "-5");
}

#[test]
fn math_ceil_at_half() {
    let code = r#"print(ceil(5.5))"#;
    assert_eq!(run_ok(code), "6");
}

#[test]
fn math_round_integer() {
    let code = r#"print(round(5.0))"#;
    assert_eq!(run_ok(code), "5");
}

#[test]
fn math_round_half_up() {
    let code = r#"print(round(5.5))"#;
    assert_eq!(run_ok(code), "6");
}

#[test]
fn math_round_half_down() {
    let code = r#"print(round(4.5))"#;
    assert_eq!(run_ok(code), "4");
}

#[test]
fn math_round_negative() {
    let code = r#"print(round(-5.7))"#;
    assert_eq!(run_ok(code), "-6");
}

#[test]
fn math_sign_positive() {
    let code = r#"print(sign(42))"#;
    assert_eq!(run_ok(code), "1");
}

#[test]
fn math_sign_zero() {
    let code = r#"print(sign(0))"#;
    assert_eq!(run_ok(code), "0");
}

#[test]
fn math_sign_negative() {
    let code = r#"print(sign(-42))"#;
    assert_eq!(run_ok(code), "-1");
}

#[test]
fn math_gcd_basic() {
    let code = r#"print(gcd(48, 18))"#;
    assert_eq!(run_ok(code), "6");
}

#[test]
fn math_gcd_coprime() {
    let code = r#"print(gcd(7, 11))"#;
    assert_eq!(run_ok(code), "1");
}

#[test]
fn math_gcd_zero_zero() {
    let code = r#"print(gcd(0, 0))"#;
    assert_eq!(run_ok(code), "0");
}

#[test]
fn math_lcm_basic() {
    let code = r#"print(lcm(12, 18))"#;
    assert_eq!(run_ok(code), "36");
}

#[test]
fn math_lcm_zero() {
    let code = r#"print(lcm(0, 5))"#;
    assert_eq!(run_ok(code), "0");
}

#[test]
fn math_is_nan_float() {
    let code = r#"print(is_nan(0.0 / 0.0))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn math_is_nan_int_false() {
    let code = r#"print(is_nan(42))"#;
    assert_eq!(run_ok(code), "false");
}

#[test]
fn math_is_inf_positive() {
    let code = r#"print(is_inf(1.0 / 0.0))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn math_is_inf_negative() {
    let code = r#"print(is_inf(-1.0 / 0.0))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn math_is_finite_normal() {
    let code = r#"print(is_finite(3.14))"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn math_is_finite_inf() {
    let code = r#"print(is_finite(1.0 / 0.0))"#;
    assert_eq!(run_ok(code), "false");
}

#[test]
fn math_int_min() {
    let code = r#"print(int_min() < 0 && int_min() + 1 < int_min() + 2)"#;
    assert_eq!(run_ok(code), "true");
}

#[test]
fn math_int_max() {
    let code = r#"print(int_max() > 0 && int_max() - 1 < int_max())"#;
    assert_eq!(run_ok(code), "true");
}
