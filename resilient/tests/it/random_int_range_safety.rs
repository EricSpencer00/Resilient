//! RES-4483: public random integer APIs must accept the full signed range.

#[test]
fn flat_random_int_accepts_full_signed_range() {
    let result = resilient::run_program(
        r#"
        fn main() {
            let value = random_int(-9223372036854775807, 9223372036854775807);
            if value >= -9223372036854775807 {
                if value < 9223372036854775807 {
                    print("ok");
                } else {
                    print("out-of-range");
                }
            } else {
                print("out-of-range");
            }
        }
        main();
        "#,
    );
    assert!(result.ok, "flat random_int failed: {:?}", result.errors);
    assert_eq!(result.stdout, "ok");
}

#[test]
fn std_random_int_accepts_full_signed_range() {
    let result = resilient::run_program(
        r#"
        use std::random;
        fn main() {
            let value = random_int(-9223372036854775807, 9223372036854775807);
            if value >= -9223372036854775807 {
                if value < 9223372036854775807 {
                    print("ok");
                } else {
                    print("out-of-range");
                }
            } else {
                print("out-of-range");
            }
        }
        main();
        "#,
    );
    assert!(result.ok, "std::random::int failed: {:?}", result.errors);
    assert_eq!(result.stdout, "ok");
}
