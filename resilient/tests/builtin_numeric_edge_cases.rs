//! Comprehensive tests for numeric builtins: checked_*, saturating_*, div_euclid,
//! rem_euclid, ilog2, ilog10, float_to_bits, float_from_bits.

#[test]
fn checked_add_overflow_i64_max() {
    let code = r#"
fn main() {
    let x = 9223372036854775807;  // i64::MAX
    let y = 1;
    match checked_add(x, y) {
        Option::Some(v) => print("overflow"),
        Option::None => print("none"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("none"));
}

#[test]
fn checked_add_success() {
    let code = r#"
fn main() {
    let x = 100;
    let y = 50;
    match checked_add(x, y) {
        Option::Some(v) => print(v),
        Option::None => print("none"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("150"));
}

#[test]
fn checked_sub_negative_success() {
    let code = r#"
fn main() {
    let x = -50;
    let y = -30;
    match checked_sub(x, y) {
        Option::Some(v) => print(v),
        Option::None => print("none"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("-20"));
}

#[test]
fn checked_sub_success() {
    let code = r#"
fn main() {
    let x = 100;
    let y = 50;
    match checked_sub(x, y) {
        Option::Some(v) => print(v),
        Option::None => print("none"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("50"));
}

#[test]
fn checked_mul_overflow() {
    let code = r#"
fn main() {
    let x = 9223372036854775807;  // i64::MAX
    let y = 2;
    match checked_mul(x, y) {
        Option::Some(v) => print("ok"),
        Option::None => print("none"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("none"));
}

#[test]
fn checked_mul_success() {
    let code = r#"
fn main() {
    let x = 100;
    let y = 50;
    match checked_mul(x, y) {
        Option::Some(v) => print(v),
        Option::None => print("none"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("5000"));
}

#[test]
fn checked_div_by_zero() {
    let code = r#"
fn main() {
    let x = 100;
    let y = 0;
    match checked_div(x, y) {
        Option::Some(v) => print("ok"),
        Option::None => print("none"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("none"));
}

#[test]
fn checked_div_success() {
    let code = r#"
fn main() {
    let x = 100;
    let y = 4;
    match checked_div(x, y) {
        Option::Some(v) => print(v),
        Option::None => print("none"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("25"));
}

#[test]
fn saturating_add_overflow() {
    let code = r#"
fn main() {
    let x = 9223372036854775807;  // i64::MAX
    let y = 1;
    print(saturating_add(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("9223372036854775807"));
}

#[test]
fn saturating_add_success() {
    let code = r#"
fn main() {
    let x = 100;
    let y = 50;
    print(saturating_add(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("150"));
}

#[test]
fn saturating_sub_success() {
    let code = r#"
fn main() {
    let x = 100;
    let y = 50;
    print(saturating_sub(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("50"));
}

#[test]
fn saturating_mul_overflow() {
    let code = r#"
fn main() {
    let x = 9223372036854775807;  // i64::MAX
    let y = 2;
    print(saturating_mul(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("9223372036854775807"));
}

#[test]
fn saturating_mul_success() {
    let code = r#"
fn main() {
    let x = 100;
    let y = 50;
    print(saturating_mul(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("5000"));
}

#[test]
fn div_euclid_positive_divisor() {
    let code = r#"
fn main() {
    let x = 17;
    let y = 5;
    print(div_euclid(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("3"));
}

#[test]
fn div_euclid_negative_dividend() {
    let code = r#"
fn main() {
    let x = -17;
    let y = 5;
    print(div_euclid(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("-4"));
}

#[test]
fn div_euclid_negative_divisor() {
    let code = r#"
fn main() {
    let x = 17;
    let y = -5;
    print(div_euclid(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("-3"));
}

#[test]
fn div_euclid_both_negative() {
    let code = r#"
fn main() {
    let x = -17;
    let y = -5;
    print(div_euclid(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("4"));
}

#[test]
fn div_euclid_by_zero_error() {
    let code = r#"
fn main() {
    let x = 100;
    let y = 0;
    print(div_euclid(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(!result.ok, "Expected error for div_euclid by zero");
    assert!(result.errors.iter().any(|e| e.contains("division by zero")));
}

#[test]
fn rem_euclid_positive() {
    let code = r#"
fn main() {
    let x = 17;
    let y = 5;
    print(rem_euclid(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("2"));
}

#[test]
fn rem_euclid_negative_dividend() {
    let code = r#"
fn main() {
    let x = -17;
    let y = 5;
    print(rem_euclid(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("3"));
}

#[test]
fn rem_euclid_negative_divisor() {
    let code = r#"
fn main() {
    let x = 17;
    let y = -5;
    print(rem_euclid(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("2"));
}

#[test]
fn rem_euclid_by_zero_error() {
    let code = r#"
fn main() {
    let x = 100;
    let y = 0;
    print(rem_euclid(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(!result.ok, "Expected error for rem_euclid by zero");
    assert!(result.errors.iter().any(|e| e.contains("division by zero")));
}

#[test]
fn ilog2_power_of_two() {
    let code = r#"
fn main() {
    print(ilog2(16));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("4"));
}

#[test]
fn ilog2_one() {
    let code = r#"
fn main() {
    print(ilog2(1));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("0"));
}

#[test]
fn ilog2_two() {
    let code = r#"
fn main() {
    print(ilog2(2));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("1"));
}

#[test]
fn ilog2_zero_error() {
    let code = r#"
fn main() {
    print(ilog2(0));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(!result.ok, "Expected error for ilog2(0)");
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.contains("domain error") || e.contains("positive"))
    );
}

#[test]
fn ilog2_negative_error() {
    let code = r#"
fn main() {
    print(ilog2(-5));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(!result.ok, "Expected error for ilog2 negative");
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.contains("domain error") || e.contains("positive"))
    );
}

#[test]
fn ilog2_i64_max() {
    let code = r#"
fn main() {
    let x = 9223372036854775807;  // i64::MAX
    print(ilog2(x));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("62"));
}

#[test]
fn ilog10_one() {
    let code = r#"
fn main() {
    print(ilog10(1));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("0"));
}

#[test]
fn ilog10_ten() {
    let code = r#"
fn main() {
    print(ilog10(10));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("1"));
}

#[test]
fn ilog10_hundred() {
    let code = r#"
fn main() {
    print(ilog10(100));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("2"));
}

#[test]
fn ilog10_zero_error() {
    let code = r#"
fn main() {
    print(ilog10(0));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(!result.ok, "Expected error for ilog10(0)");
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.contains("domain error") || e.contains("positive"))
    );
}

#[test]
fn ilog10_negative_error() {
    let code = r#"
fn main() {
    print(ilog10(-10));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(!result.ok, "Expected error for ilog10 negative");
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.contains("domain error") || e.contains("positive"))
    );
}

#[test]
fn ilog10_i64_max() {
    let code = r#"
fn main() {
    let x = 9223372036854775807;  // i64::MAX
    print(ilog10(x));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("18"));
}

#[test]
fn float_to_bits_simple() {
    let code = r#"
fn main() {
    let f = 1.0;
    let bits = float_to_bits(f);
    print(bits);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("4607182418800017408"));
}

#[test]
fn float_to_bits_zero() {
    let code = r#"
fn main() {
    let f = 0.0;
    let bits = float_to_bits(f);
    print(bits);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("0"));
}

#[test]
fn float_from_bits_roundtrip_one() {
    let code = r#"
fn main() {
    let f = 1.0;
    let bits = float_to_bits(f);
    let f2 = float_from_bits(bits);
    if f == f2 {
        print("roundtrip");
    } else {
        print("failed");
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("roundtrip"));
}

#[test]
fn float_from_bits_zero() {
    let code = r#"
fn main() {
    let bits = 0;
    let f = float_from_bits(bits);
    if f == 0.0 {
        print("zero");
    } else {
        print("not_zero");
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("zero"));
}

#[test]
fn checked_add_negative_numbers() {
    let code = r#"
fn main() {
    let x = -100;
    let y = -50;
    match checked_add(x, y) {
        Option::Some(v) => print(v),
        Option::None => print("none"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("-150"));
}

#[test]
fn div_euclid_large_numbers() {
    let code = r#"
fn main() {
    let x = 1000000000;
    let y = 3;
    print(div_euclid(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("333333333"));
}

#[test]
fn rem_euclid_always_non_negative() {
    let code = r#"
fn main() {
    let x = -17;
    let y = 5;
    let r = rem_euclid(x, y);
    if r >= 0 {
        print("non-negative");
    } else {
        print("negative");
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("non-negative"));
}

#[test]
fn ilog2_nearby_power_of_two() {
    let code = r#"
fn main() {
    print(ilog2(1023));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("9"));
}

#[test]
fn ilog2_just_above_power_of_two() {
    let code = r#"
fn main() {
    print(ilog2(1025));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("10"));
}

#[test]
fn checked_div_negative_dividend() {
    let code = r#"
fn main() {
    let x = -100;
    let y = 4;
    match checked_div(x, y) {
        Option::Some(v) => print(v),
        Option::None => print("none"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("-25"));
}

#[test]
fn checked_div_negative_divisor() {
    let code = r#"
fn main() {
    let x = 100;
    let y = -4;
    match checked_div(x, y) {
        Option::Some(v) => print(v),
        Option::None => print("none"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("-25"));
}

#[test]
fn ilog10_ninety_nine() {
    let code = r#"
fn main() {
    print(ilog10(99));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("1"));
}

#[test]
fn ilog10_one_thousand() {
    let code = r#"
fn main() {
    print(ilog10(1000));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("3"));
}

#[test]
fn saturating_add_zero() {
    let code = r#"
fn main() {
    let x = 0;
    let y = 100;
    print(saturating_add(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("100"));
}

#[test]
fn saturating_sub_zero() {
    let code = r#"
fn main() {
    let x = 100;
    let y = 0;
    print(saturating_sub(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("100"));
}

#[test]
fn saturating_mul_by_zero() {
    let code = r#"
fn main() {
    let x = 100;
    let y = 0;
    print(saturating_mul(x, y));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("0"));
}

#[test]
fn checked_mul_by_one() {
    let code = r#"
fn main() {
    let x = 12345;
    let y = 1;
    match checked_mul(x, y) {
        Option::Some(v) => print(v),
        Option::None => print("none"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("12345"));
}
