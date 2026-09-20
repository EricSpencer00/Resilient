use resilient_runtime::fixed::Fixed;

#[test]
fn from_raw_neutralizes_invalid_widths() {
    assert_eq!(Fixed::<8, 16>::from_raw(123).raw(), 0);
    assert_eq!(Fixed::<u32::MAX, 1>::from_raw(-456).raw(), 0);
}

#[test]
fn from_raw_preserves_unchecked_values_for_valid_widths() {
    assert_eq!(Fixed::<16, 16>::from_raw(i64::MAX).raw(), i64::MAX);
    assert_eq!(Fixed::<32, 32>::from_raw(i64::MIN).raw(), i64::MIN);
}
