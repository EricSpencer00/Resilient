use resilient_runtime::fixed::Fixed;

#[test]
fn invalid_width_metadata_does_not_overflow_at_compile_time() {
    const OVERFLOWED_WIDTH: u32 = Fixed::<{ u32::MAX }, 1>::TOTAL_BITS;

    assert_eq!(OVERFLOWED_WIDTH, 0);
    assert!(Fixed::<{ u32::MAX }, 1>::new(0).is_none());
}

#[test]
fn valid_width_metadata_is_unchanged() {
    assert_eq!(Fixed::<16, 16>::TOTAL_BITS, 32);
    assert_eq!(Fixed::<32, 32>::TOTAL_BITS, 64);
}
