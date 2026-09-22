//! Checked cardinality for first-class integer ranges.

/// Return the cardinality of an integer range when it fits in `Int`.
///
/// Reversed ranges preserve the existing `len` convention of producing zero.
/// The intermediate arithmetic uses `i128` because the difference between two
/// `i64` endpoints can be one larger than the largest representable `Int`.
pub(crate) fn checked_range_len(start: i64, end: i64, inclusive: bool) -> Option<i64> {
    if end < start || (!inclusive && end == start) {
        return Some(0);
    }

    let span = i128::from(end) - i128::from(start);
    let count = if inclusive { span + 1 } else { span };
    i64::try_from(count).ok()
}

#[cfg(test)]
mod tests {
    use super::checked_range_len;
    use crate::{Value, builtin_len};

    #[test]
    fn preserves_valid_boundary_length() {
        assert_eq!(checked_range_len(0, i64::MAX, false), Some(i64::MAX));
        assert_eq!(checked_range_len(0, i64::MAX - 1, true), Some(i64::MAX));
    }

    #[test]
    fn keeps_reversed_and_empty_ranges_zero() {
        assert_eq!(checked_range_len(4, 4, false), Some(0));
        assert_eq!(checked_range_len(4, 3, true), Some(0));
    }

    #[test]
    fn rejects_range_cardinality_that_exceeds_int() {
        assert_eq!(checked_range_len(i64::MIN, i64::MAX, false), None);
        assert_eq!(checked_range_len(i64::MIN, i64::MAX, true), None);
    }

    #[test]
    fn builtin_len_reports_range_overflow_as_typed_error() {
        for inclusive in [false, true] {
            let err = builtin_len(&[Value::Range {
                start: i64::MIN,
                end: i64::MAX,
                inclusive,
            }])
            .expect_err("unrepresentable range length must fail");
            assert!(err.contains("cardinality exceeds Int::MAX"), "{err}");
        }
    }
}
