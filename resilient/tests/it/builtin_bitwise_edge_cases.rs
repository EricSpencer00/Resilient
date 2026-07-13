//! Comprehensive edge-case tests for bitwise builtins in Resilient.
//! Covers: bit_and, bit_or, bit_xor, bit_not, bit_shl, bit_shr,
//! bit_rotate_left, bit_rotate_right, bit_count, bit_leading_zeros,
//! bit_trailing_zeros, bit_test, bit_set, bit_clear, bit_toggle,
//! and bytes_* operations.

fn run_ok(src: &str) -> String {
    let r = resilient::run_program(src);
    assert!(r.ok, "failed: {:?}", r.errors);
    r.stdout
}

fn run_err(src: &str) -> String {
    let r = resilient::run_program(src);
    assert!(!r.ok, "expected error but succeeded");
    r.errors.join("\n")
}

// ============================================================================
// bit_and tests
// ============================================================================

#[test]
fn bit_and_basic() {
    let out = run_ok(r#"fn main() { println(bit_and(0xFF, 0x0F)); } main();"#);
    assert_eq!(out.trim(), "15");
}

#[test]
fn bit_and_zero() {
    let out = run_ok(r#"fn main() { println(bit_and(0xFFFF, 0)); } main();"#);
    assert_eq!(out.trim(), "0");
}

#[test]
fn bit_and_negative_one() {
    let out = run_ok(r#"fn main() { println(bit_and(-1, 42)); } main();"#);
    assert_eq!(out.trim(), "42");
}

#[test]
fn bit_and_both_negative() {
    let out = run_ok(r#"fn main() { println(bit_and(-1, -2)); } main();"#);
    assert_eq!(out.trim(), "-2");
}

// ============================================================================
// bit_or tests
// ============================================================================

#[test]
fn bit_or_basic() {
    let out = run_ok(r#"fn main() { println(bit_or(0xF0, 0x0F)); } main();"#);
    assert_eq!(out.trim(), "255");
}

#[test]
fn bit_or_zero() {
    let out = run_ok(r#"fn main() { println(bit_or(0, 0)); } main();"#);
    assert_eq!(out.trim(), "0");
}

#[test]
fn bit_or_with_one() {
    let out =
        run_ok(r#"fn main() { let minval = bit_shl(1, 63); println(bit_or(minval, 1)); } main();"#);
    assert_eq!(out.trim(), "-9223372036854775807");
}

// ============================================================================
// bit_xor tests
// ============================================================================

#[test]
fn bit_xor_basic() {
    let out = run_ok(r#"fn main() { println(bit_xor(0xFF, 0x0F)); } main();"#);
    assert_eq!(out.trim(), "240");
}

#[test]
fn bit_xor_self_is_zero() {
    let out = run_ok(r#"fn main() { println(bit_xor(42, 42)); } main();"#);
    assert_eq!(out.trim(), "0");
}

#[test]
fn bit_xor_negative_one() {
    let out = run_ok(r#"fn main() { println(bit_xor(-1, 0)); } main();"#);
    assert_eq!(out.trim(), "-1");
}

// ============================================================================
// bit_not tests
// ============================================================================

#[test]
fn bit_not_zero() {
    let out = run_ok(r#"fn main() { println(bit_not(0)); } main();"#);
    assert_eq!(out.trim(), "-1");
}

#[test]
fn bit_not_negative_one() {
    let out = run_ok(r#"fn main() { println(bit_not(-1)); } main();"#);
    assert_eq!(out.trim(), "0");
}

#[test]
fn bit_not_involution() {
    let out = run_ok(r#"fn main() { println(bit_not(bit_not(42))); } main();"#);
    assert_eq!(out.trim(), "42");
}

// ============================================================================
// bit_shl tests (shift left)
// ============================================================================

#[test]
fn bit_shl_zero_shift() {
    let out = run_ok(r#"fn main() { println(bit_shl(1, 0)); } main();"#);
    assert_eq!(out.trim(), "1");
}

#[test]
fn bit_shl_basic() {
    let out = run_ok(r#"fn main() { println(bit_shl(1, 3)); } main();"#);
    assert_eq!(out.trim(), "8");
}

#[test]
fn bit_shl_one_shifts() {
    let out = run_ok(r#"fn main() { println(bit_shl(1, 1)); } main();"#);
    assert_eq!(out.trim(), "2");
}

#[test]
fn bit_shl_max_shift() {
    let out = run_ok(r#"fn main() { println(bit_shl(1, 63)); } main();"#);
    assert_eq!(out.trim(), "-9223372036854775808");
}

#[test]
fn bit_shl_negative_one() {
    let out = run_ok(r#"fn main() { println(bit_shl(-1, 1)); } main();"#);
    assert_eq!(out.trim(), "-2");
}

#[test]
fn bit_shl_shift_64_errors() {
    let out = run_err(r#"fn main() { println(bit_shl(1, 64)); } main();"#);
    assert!(
        out.contains("0..=63"),
        "Expected shift range error, got: {}",
        out
    );
}

#[test]
fn bit_shl_negative_shift_errors() {
    let out = run_err(r#"fn main() { println(bit_shl(1, -1)); } main();"#);
    assert!(
        out.contains("0..=63"),
        "Expected shift range error, got: {}",
        out
    );
}

// ============================================================================
// bit_shr tests (shift right, arithmetic)
// ============================================================================

#[test]
fn bit_shr_zero_shift() {
    let out = run_ok(r#"fn main() { println(bit_shr(8, 0)); } main();"#);
    assert_eq!(out.trim(), "8");
}

#[test]
fn bit_shr_basic() {
    let out = run_ok(r#"fn main() { println(bit_shr(16, 2)); } main();"#);
    assert_eq!(out.trim(), "4");
}

#[test]
fn bit_shr_one_shift() {
    let out = run_ok(r#"fn main() { println(bit_shr(7, 1)); } main();"#);
    assert_eq!(out.trim(), "3");
}

#[test]
fn bit_shr_max_shift() {
    let out = run_ok(r#"fn main() { println(bit_shr(-1, 63)); } main();"#);
    assert_eq!(out.trim(), "-1");
}

#[test]
fn bit_shr_arithmetic_preserves_sign() {
    let out = run_ok(r#"fn main() { println(bit_shr(-8, 1)); } main();"#);
    assert_eq!(out.trim(), "-4");
}

#[test]
fn bit_shr_shift_64_errors() {
    let out = run_err(r#"fn main() { println(bit_shr(1, 64)); } main();"#);
    assert!(
        out.contains("0..=63"),
        "Expected shift range error, got: {}",
        out
    );
}

#[test]
fn bit_shr_negative_shift_errors() {
    let out = run_err(r#"fn main() { println(bit_shr(1, -1)); } main();"#);
    assert!(
        out.contains("0..=63"),
        "Expected shift range error, got: {}",
        out
    );
}

// ============================================================================
// bit_rotate_left tests
// ============================================================================

#[test]
fn bit_rotate_left_zero_rotation() {
    let out = run_ok(
        r#"fn main() { let x = 1311768467463008496; println(bit_rotate_left(x, 0)); } main();"#,
    );
    assert_eq!(out.trim(), "1311768467463008496");
}

#[test]
fn bit_rotate_left_basic() {
    let out = run_ok(r#"fn main() { println(bit_rotate_left(1, 1)); } main();"#);
    assert_eq!(out.trim(), "2");
}

#[test]
fn bit_rotate_left_wraps() {
    let out = run_ok(
        r#"fn main() { let minval = bit_shl(1, 63); println(bit_rotate_left(minval, 1)); } main();"#,
    );
    assert_eq!(out.trim(), "1");
}

#[test]
fn bit_rotate_left_max_rotation() {
    let out = run_ok(r#"fn main() { println(bit_rotate_left(42, 63)); } main();"#);
    assert_eq!(out.trim(), "21");
}

#[test]
fn bit_rotate_left_rotate_64_errors() {
    let out = run_err(r#"fn main() { println(bit_rotate_left(1, 64)); } main();"#);
    assert!(
        out.contains("0..=63"),
        "Expected rotation range error, got: {}",
        out
    );
}

// ============================================================================
// bit_rotate_right tests
// ============================================================================

#[test]
fn bit_rotate_right_zero_rotation() {
    let out = run_ok(
        r#"fn main() { let x = 1311768467463008496; println(bit_rotate_right(x, 0)); } main();"#,
    );
    assert_eq!(out.trim(), "1311768467463008496");
}

#[test]
fn bit_rotate_right_basic() {
    let out = run_ok(r#"fn main() { println(bit_rotate_right(2, 1)); } main();"#);
    assert_eq!(out.trim(), "1");
}

#[test]
fn bit_rotate_right_wraps() {
    let out = run_ok(r#"fn main() { println(bit_rotate_right(1, 1)); } main();"#);
    assert_eq!(out.trim(), "-9223372036854775808"); // LSB wraps to MSB
}

#[test]
fn bit_rotate_right_max_rotation() {
    let out = run_ok(r#"fn main() { println(bit_rotate_right(42, 63)); } main();"#);
    assert_eq!(out.trim(), "84");
}

#[test]
fn bit_rotate_right_rotate_64_errors() {
    let out = run_err(r#"fn main() { println(bit_rotate_right(1, 64)); } main();"#);
    assert!(
        out.contains("0..=63"),
        "Expected rotation range error, got: {}",
        out
    );
}

// ============================================================================
// bit_count tests (population count / Hamming weight)
// ============================================================================

#[test]
fn bit_count_zero() {
    let out = run_ok(r#"fn main() { println(bit_count(0)); } main();"#);
    assert_eq!(out.trim(), "0");
}

#[test]
fn bit_count_one() {
    let out = run_ok(r#"fn main() { println(bit_count(1)); } main();"#);
    assert_eq!(out.trim(), "1");
}

#[test]
fn bit_count_negative_one() {
    let out = run_ok(r#"fn main() { println(bit_count(-1)); } main();"#);
    assert_eq!(out.trim(), "64");
}

#[test]
fn bit_count_all_bits() {
    let out = run_ok(r#"fn main() { println(bit_count(-1)); } main();"#);
    assert_eq!(out.trim(), "64");
}

#[test]
fn bit_count_alternating() {
    // Pattern: alternate 1 and 0 bits: 0xAAAAAAAAAAAAAAAA
    // We create this by XORing -1 with a shifted version
    let out = run_ok(
        r#"fn main() { let x = bit_xor(-1, 0x5555555555555555); println(bit_count(x)); } main();"#,
    );
    assert_eq!(out.trim(), "32");
}

// ============================================================================
// bit_leading_zeros tests
// ============================================================================

#[test]
fn bit_leading_zeros_zero() {
    let out = run_ok(r#"fn main() { println(bit_leading_zeros(0)); } main();"#);
    assert_eq!(out.trim(), "64");
}

#[test]
fn bit_leading_zeros_one() {
    let out = run_ok(r#"fn main() { println(bit_leading_zeros(1)); } main();"#);
    assert_eq!(out.trim(), "63");
}

#[test]
fn bit_leading_zeros_negative_one() {
    let out = run_ok(r#"fn main() { println(bit_leading_zeros(-1)); } main();"#);
    assert_eq!(out.trim(), "0");
}

#[test]
fn bit_leading_zeros_max_positive() {
    let out = run_ok(r#"fn main() { println(bit_leading_zeros(9223372036854775807)); } main();"#);
    assert_eq!(out.trim(), "1");
}

#[test]
fn bit_leading_zeros_msb_set() {
    let out = run_ok(
        r#"fn main() { let minval = bit_shl(1, 63); println(bit_leading_zeros(minval)); } main();"#,
    );
    assert_eq!(out.trim(), "0");
}

// ============================================================================
// bit_trailing_zeros tests
// ============================================================================

#[test]
fn bit_trailing_zeros_zero() {
    let out = run_ok(r#"fn main() { println(bit_trailing_zeros(0)); } main();"#);
    assert_eq!(out.trim(), "64");
}

#[test]
fn bit_trailing_zeros_one() {
    let out = run_ok(r#"fn main() { println(bit_trailing_zeros(1)); } main();"#);
    assert_eq!(out.trim(), "0");
}

#[test]
fn bit_trailing_zeros_two() {
    let out = run_ok(r#"fn main() { println(bit_trailing_zeros(2)); } main();"#);
    assert_eq!(out.trim(), "1");
}

#[test]
fn bit_trailing_zeros_negative_one() {
    let out = run_ok(r#"fn main() { println(bit_trailing_zeros(-1)); } main();"#);
    assert_eq!(out.trim(), "0");
}

#[test]
fn bit_trailing_zeros_min_int() {
    let out = run_ok(
        r#"fn main() { let minval = bit_shl(1, 63); println(bit_trailing_zeros(minval)); } main();"#,
    );
    assert_eq!(out.trim(), "63");
}

// ============================================================================
// bit_test tests
// ============================================================================

#[test]
fn bit_test_basic_true() {
    let out = run_ok(r#"fn main() { println(bit_test(1, 0)); } main();"#);
    assert_eq!(out.trim(), "true");
}

#[test]
fn bit_test_basic_false() {
    let out = run_ok(r#"fn main() { println(bit_test(2, 0)); } main();"#);
    assert_eq!(out.trim(), "false");
}

#[test]
fn bit_test_msb() {
    let out = run_ok(r#"fn main() { println(bit_test(-1, 63)); } main();"#);
    assert_eq!(out.trim(), "true");
}

// ============================================================================
// bit_set tests
// ============================================================================

#[test]
fn bit_set_zero() {
    let out = run_ok(r#"fn main() { println(bit_set(0, 0)); } main();"#);
    assert_eq!(out.trim(), "1");
}

#[test]
fn bit_set_already_set() {
    let out = run_ok(r#"fn main() { println(bit_set(1, 0)); } main();"#);
    assert_eq!(out.trim(), "1");
}

// ============================================================================
// bit_clear tests
// ============================================================================

#[test]
fn bit_clear_one() {
    let out = run_ok(r#"fn main() { println(bit_clear(1, 0)); } main();"#);
    assert_eq!(out.trim(), "0");
}

#[test]
fn bit_clear_already_clear() {
    let out = run_ok(r#"fn main() { println(bit_clear(0, 0)); } main();"#);
    assert_eq!(out.trim(), "0");
}

// ============================================================================
// bit_toggle tests
// ============================================================================

#[test]
fn bit_toggle_zero() {
    let out = run_ok(r#"fn main() { println(bit_toggle(0, 0)); } main();"#);
    assert_eq!(out.trim(), "1");
}

#[test]
fn bit_toggle_one() {
    let out = run_ok(r#"fn main() { println(bit_toggle(1, 0)); } main();"#);
    assert_eq!(out.trim(), "0");
}

// ============================================================================
// bytes operations
// ============================================================================

#[test]
fn bytes_xor_basic() {
    let out = run_ok(
        r#"
fn main() {
    let a = b"\xFF\x0F";
    let b = b"\xAA\xAA";
    let result = bytes_xor(a, b);
    println(bytes_len(result));
}
main();"#,
    );
    assert_eq!(out.trim(), "2");
}

#[test]
fn bytes_and_basic() {
    let out = run_ok(
        r#"
fn main() {
    let a = b"\xFF\xFF";
    let b = b"\x0F\xF0";
    let result = bytes_and(a, b);
    println(bytes_len(result));
}
main();"#,
    );
    assert_eq!(out.trim(), "2");
}

#[test]
fn bytes_or_basic() {
    let out = run_ok(
        r#"
fn main() {
    let a = b"\xF0\x20";
    let b = b"\x0F\x0F";
    let result = bytes_or(a, b);
    println(bytes_len(result));
}
main();"#,
    );
    assert_eq!(out.trim(), "2");
}

#[test]
fn bytes_not_basic() {
    let out = run_ok(
        r#"
fn main() {
    let a = b"\x00\xFF";
    let result = bytes_not(a);
    println(bytes_len(result));
}
main();"#,
    );
    assert_eq!(out.trim(), "2");
}

#[test]
fn bytes_reverse_basic() {
    let out = run_ok(
        r#"
fn main() {
    let a = b"\x01\x02\x03";
    let result = bytes_reverse(a);
    println(bytes_len(result));
}
main();"#,
    );
    assert_eq!(out.trim(), "3");
}

#[test]
fn bytes_fill_basic() {
    let out = run_ok(
        r#"
fn main() {
    let result = bytes_fill(5, 42);
    println(bytes_len(result));
}
main();"#,
    );
    assert_eq!(out.trim(), "5");
}
