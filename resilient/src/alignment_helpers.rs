//! RES-1136: alignment helpers — `next_multiple_of` and `is_multiple_of`.
//!
//! Buffer padding, DMA descriptor alignment, SIMD lane sizing, and
//! ring-buffer power-of-two indexing all need to round counts up to an
//! alignment boundary or test divisibility. These two pure leaf builtins
//! replace the inline `if n % m == 0 { n } else { n + (m - n % m) }`
//! pattern with explicit, sign-correct, overflow-checked primitives.
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `next_multiple_of(n, m)` | `(Int, Int) -> Int` | Smallest multiple of `m` that is ≥ `n` |
//! | `is_multiple_of(a, b)`   | `(Int, Int) -> Bool` | `true` iff `a` is an exact multiple of `b` |
//!
//! Both are deterministic, allocate nothing, and never panic — every
//! error path returns a typed `RResult` diagnostic.

use crate::{RResult, Value};

/// `next_multiple_of(n, m) -> Int` — smallest multiple of `m` that is
/// greater than or equal to `n`. `m` must be **positive**; `m ≤ 0` is
/// a typed error (alignment is always positive in practice, and
/// negative-`m` semantics would be ambiguous). On overflow, surfaces a
/// typed error rather than wrapping silently.
///
/// Examples (all with `m = 4`):
/// - `n = 0` → `0`
/// - `n = 1` → `4`
/// - `n = 4` → `4`
/// - `n = 5` → `8`
/// - `n = -1` → `0` (smallest multiple of 4 that is ≥ -1)
/// - `n = -5` → `-4`
pub(crate) fn builtin_next_multiple_of(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n), Value::Int(m)] => {
            if *m <= 0 {
                return Err(format!(
                    "next_multiple_of: alignment must be positive, got {}",
                    m
                ));
            }
            // Rust's `%` returns a remainder with the sign of the
            // dividend, so `(m - n%m) % m` gives the (non-negative)
            // distance to the next multiple. Adding it to `n` lifts
            // `n` up to (or keeps it at) that multiple.
            let r = n.rem_euclid(*m);
            if r == 0 {
                return Ok(Value::Int(*n));
            }
            let bump = *m - r;
            match n.checked_add(bump) {
                Some(v) => Ok(Value::Int(v)),
                None => Err(format!(
                    "next_multiple_of: overflow — {} + {} exceeds i64::MAX",
                    n, bump
                )),
            }
        }
        [a, b] => Err(format!(
            "next_multiple_of: expected (int, int), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "next_multiple_of: expected 2 arguments (n, m), got {}",
            args.len()
        )),
    }
}

/// `is_multiple_of(a, b) -> Bool` — true iff `a` is an exact multiple
/// of `b`. `b == 0` is a typed error (division by zero); negative `b`
/// is fine and is treated the same as `|b|` — every nonzero integer
/// divides the same multiples of itself as its negation.
pub(crate) fn builtin_is_multiple_of(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(a), Value::Int(b)] => {
            if *b == 0 {
                return Err("is_multiple_of: divisor must be non-zero".to_string());
            }
            // i64::MIN % -1 would overflow if we used the operator,
            // but i64::MIN IS a multiple of -1, so short-circuit.
            if *a == i64::MIN && *b == -1 {
                return Ok(Value::Bool(true));
            }
            Ok(Value::Bool(a % b == 0))
        }
        [a, b] => Err(format!(
            "is_multiple_of: expected (int, int), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "is_multiple_of: expected 2 arguments (a, b), got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nm(n: i64, m: i64) -> Result<i64, String> {
        match builtin_next_multiple_of(&[Value::Int(n), Value::Int(m)])? {
            Value::Int(v) => Ok(v),
            other => panic!("expected Int, got {:?}", other),
        }
    }

    fn im(a: i64, b: i64) -> Result<bool, String> {
        match builtin_is_multiple_of(&[Value::Int(a), Value::Int(b)])? {
            Value::Bool(v) => Ok(v),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn next_multiple_aligns_positive() {
        assert_eq!(nm(0, 4).unwrap(), 0);
        assert_eq!(nm(1, 4).unwrap(), 4);
        assert_eq!(nm(3, 4).unwrap(), 4);
        assert_eq!(nm(4, 4).unwrap(), 4);
        assert_eq!(nm(5, 4).unwrap(), 8);
        assert_eq!(nm(7, 4).unwrap(), 8);
        assert_eq!(nm(8, 4).unwrap(), 8);
    }

    #[test]
    fn next_multiple_handles_negative_n() {
        // smallest multiple of 4 that is ≥ -1 is 0
        assert_eq!(nm(-1, 4).unwrap(), 0);
        // smallest multiple of 4 that is ≥ -5 is -4
        assert_eq!(nm(-5, 4).unwrap(), -4);
        // exact negative multiple — return n unchanged
        assert_eq!(nm(-8, 4).unwrap(), -8);
        // smallest multiple of 4 that is ≥ -7 is -4
        assert_eq!(nm(-7, 4).unwrap(), -4);
    }

    #[test]
    fn next_multiple_alignment_one_is_identity() {
        // every integer is a multiple of 1.
        assert_eq!(nm(0, 1).unwrap(), 0);
        assert_eq!(nm(7, 1).unwrap(), 7);
        assert_eq!(nm(-7, 1).unwrap(), -7);
        assert_eq!(nm(i64::MAX, 1).unwrap(), i64::MAX);
        assert_eq!(nm(i64::MIN, 1).unwrap(), i64::MIN);
    }

    #[test]
    fn next_multiple_rejects_zero_alignment() {
        let err = nm(5, 0).unwrap_err();
        assert!(err.contains("alignment must be positive"));
    }

    #[test]
    fn next_multiple_rejects_negative_alignment() {
        let err = nm(5, -4).unwrap_err();
        assert!(err.contains("alignment must be positive"));
    }

    #[test]
    fn next_multiple_detects_overflow() {
        // Anything just below i64::MAX that doesn't already align
        // forces an overflow when bumped up to the next multiple.
        let err = nm(i64::MAX, 4).unwrap_err();
        assert!(err.contains("overflow"), "got {}", err);
        let err = nm(i64::MAX - 1, 4).unwrap_err();
        assert!(err.contains("overflow"));
    }

    #[test]
    fn next_multiple_max_minus_three_aligned_to_four_works() {
        // i64::MAX = 9223372036854775807. MAX-3 % 4 = ?
        // 9223372036854775807 mod 4 = 3 (since MAX = 4q + 3).
        // So MAX-3 mod 4 = 0 — already aligned, returns MAX-3.
        assert_eq!(nm(i64::MAX - 3, 4).unwrap(), i64::MAX - 3);
    }

    #[test]
    fn next_multiple_rejects_wrong_arity_and_types() {
        let err = builtin_next_multiple_of(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected 2"));
        let err = builtin_next_multiple_of(&[Value::Int(1), Value::Bool(true)]).unwrap_err();
        assert!(err.contains("expected (int, int)"));
    }

    #[test]
    fn next_multiple_typical_alignment_values() {
        // 8-byte alignment of a 13-byte payload.
        assert_eq!(nm(13, 8).unwrap(), 16);
        // 64-byte cache-line alignment of 100.
        assert_eq!(nm(100, 64).unwrap(), 128);
        // 4096-byte page alignment of a counter at exactly one page.
        assert_eq!(nm(4096, 4096).unwrap(), 4096);
        // Same counter, one byte further → next page.
        assert_eq!(nm(4097, 4096).unwrap(), 8192);
    }

    #[test]
    fn is_multiple_positive_cases() {
        assert!(im(0, 5).unwrap());
        assert!(im(10, 5).unwrap());
        assert!(im(15, 5).unwrap());
        assert!(!im(7, 5).unwrap());
        assert!(!im(11, 5).unwrap());
    }

    #[test]
    fn is_multiple_negative_a() {
        assert!(im(-10, 5).unwrap());
        assert!(im(-15, 5).unwrap());
        assert!(!im(-7, 5).unwrap());
    }

    #[test]
    fn is_multiple_negative_b_same_as_positive_b() {
        // |b| works for membership in the multiples set.
        assert!(im(10, -5).unwrap());
        assert!(im(-10, -5).unwrap());
        assert!(!im(7, -5).unwrap());
    }

    #[test]
    fn is_multiple_b_eq_one_always_true() {
        assert!(im(0, 1).unwrap());
        assert!(im(42, 1).unwrap());
        assert!(im(-42, 1).unwrap());
        assert!(im(i64::MAX, 1).unwrap());
        assert!(im(i64::MIN, 1).unwrap());
    }

    #[test]
    fn is_multiple_b_eq_minus_one_always_true() {
        assert!(im(0, -1).unwrap());
        assert!(im(42, -1).unwrap());
        assert!(im(-42, -1).unwrap());
        assert!(im(i64::MAX, -1).unwrap());
        // i64::MIN % -1 would overflow with the operator — must
        // short-circuit. MIN is indeed a multiple of -1.
        assert!(im(i64::MIN, -1).unwrap());
    }

    #[test]
    fn is_multiple_rejects_zero_divisor() {
        let err = im(5, 0).unwrap_err();
        assert!(err.contains("non-zero"));
        let err = im(0, 0).unwrap_err();
        assert!(err.contains("non-zero"));
    }

    #[test]
    fn is_multiple_rejects_wrong_arity_and_types() {
        let err = builtin_is_multiple_of(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected 2"));
        let err = builtin_is_multiple_of(&[Value::Int(1), Value::Bool(true)]).unwrap_err();
        assert!(err.contains("expected (int, int)"));
    }

    #[test]
    fn round_trip_with_next_multiple_of() {
        // next_multiple_of(n, m) is itself always a multiple of m.
        for &(n, m) in &[(0i64, 4), (1, 4), (5, 4), (8, 4), (-3, 4), (100, 64)] {
            let aligned = nm(n, m).unwrap();
            assert!(
                im(aligned, m).unwrap(),
                "next_multiple_of({n}, {m}) = {aligned} is not a multiple of {m}"
            );
        }
    }
}
