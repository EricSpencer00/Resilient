//! RES-4754: byte-slice counts stay clamp-safe across target widths.

#[test]
fn oversized_counts_clamp_across_all_byte_slices() {
    let result = resilient::run_program(
        r#"
fn main() {
    let b = b"abc";
    let take = bytes_take(b, 9223372036854775807);
    let drop = bytes_drop(b, 9223372036854775807);
    let take_last = bytes_take_last(b, 9223372036854775807);
    let drop_last = bytes_drop_last(b, 9223372036854775807);
    print(bytes_eq(take, b));
    print(bytes_len(drop));
    print(bytes_eq(take_last, b));
    print(bytes_len(drop_last));
}
main();
"#,
    );

    assert!(
        result.ok,
        "byte-slice boundary program failed: {:?}",
        result.errors
    );
    assert_eq!(result.stdout, "true0true0");
}
