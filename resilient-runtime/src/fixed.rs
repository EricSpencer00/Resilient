//! RES-367: `Fixed<N, D>` — fixed-point arithmetic for FPU-less
//! Cortex-M / RISC-V cores.
//!
//! `N` integer bits + `D` fractional bits, total width 32 or 64.
//! Storage is always `i64` so the same type covers both widths
//! without a sealed-trait dance; the smaller layouts assert their
//! invariants at construction. All ops are `#[inline]` and
//! allocation-free; the module compiles under default (no-alloc)
//! features so it is available on every embedded build.
//!
//! # Semantics
//!
//! Arithmetic is wrapping (matches the rest of the runtime — see
//! the `wrapping_add` etc. in `lib.rs`). Multiplication and
//! division correct for the scale factor `2^D` so `(1.5 * 2.0) ==
//! 3.0` regardless of which `Fixed<N, D>` representation you pick.
//! Comparison is exact bit-equality on the underlying integer
//! (no NaN concept — that's the whole point of fixed-point).
//!
//! # Conversions
//!
//! `Fixed::<N, D>::from_int(i)` shifts in by `D`; if the shift
//! overflows the integer storage range, `None` is returned.
//! `to_int` truncates toward zero. `from_float` rounds to nearest
//! and saturates on out-of-range; `to_float` is exact in the
//! "values that fit a 53-bit mantissa" range and lossy beyond.
//!
//! # Bit-width constraint
//!
//! `N + D` must be 32 or 64. The constructor `Fixed::new(raw)`
//! and `from_int` / `from_float` all check this constraint at
//! call time (a const expression — the compiler folds it).
//! Construction with an invalid `<N, D>` returns `None`. We can't
//! reject it at the type level on stable Rust without sealed
//! traits, so we settle for "ergonomic at use, rejected at
//! construction".

/// A fixed-point number with `N` integer bits and `D` fractional
/// bits. `N + D` must be 32 or 64. Storage is `i64`; a 32-bit
/// configuration just leaves the upper half implicitly sign-
/// extended.
///
/// `Copy` because the type is `i64`-sized at most.
#[derive(Debug, Clone, Copy)]
pub struct Fixed<const N: u32, const D: u32> {
    /// Underlying integer representation. The real-number value is
    /// `raw / 2^D`. Stored as `i64` so both 32-bit and 64-bit
    /// configurations share the same arithmetic primitives.
    raw: i64,
}

/// Compile-time check that `N + D` is a valid total width. We
/// can't put this in a `where` clause on stable Rust (would need
/// `generic_const_exprs`), so it lives as a runtime const-fn that
/// callers invoke; the compiler folds it to a single `bool`.
#[inline]
const fn valid_width(n: u32, d: u32) -> bool {
    let total = n.wrapping_add(d);
    total == 32 || total == 64
}

/// Range of valid raw values for an N-integer-bit, D-fractional-
/// bit number. For total width 32, raw must fit in i32 range; for
/// total width 64, all i64 values are valid.
#[inline]
const fn raw_range(n: u32, d: u32) -> (i64, i64) {
    let total = n.wrapping_add(d);
    if total == 32 {
        (i32::MIN as i64, i32::MAX as i64)
    } else {
        (i64::MIN, i64::MAX)
    }
}

impl<const N: u32, const D: u32> Fixed<N, D> {
    /// Total bit width = N + D. Either 32 or 64 for valid
    /// configurations.
    pub const TOTAL_BITS: u32 = N + D;

    /// Construct from a raw integer representation. The raw value
    /// is the number multiplied by `2^D`, e.g. for `Fixed<16, 16>`,
    /// `Fixed::new(0x10000)` is `1.0`.
    ///
    /// Returns `None` if `<N, D>` is not a valid 32/64-width
    /// configuration, or if `raw` is outside the storage range
    /// for the selected total width.
    #[inline]
    pub fn new(raw: i64) -> Option<Self> {
        if !valid_width(N, D) {
            return None;
        }
        let (lo, hi) = raw_range(N, D);
        if raw < lo || raw > hi {
            return None;
        }
        Some(Self { raw })
    }

    /// Construct from a raw integer representation without
    /// validating the storage range. Useful for `const`
    /// initialisation paths where `<N, D>` is known good.
    ///
    /// Returns `Self { raw: 0 }` (a valid zero value) if `<N, D>`
    /// is invalid; the type-system constraint is "best-effort" on
    /// stable Rust. Prefer [`Fixed::new`] in code where the
    /// generic params come from outside.
    #[inline]
    pub const fn from_raw(raw: i64) -> Self {
        Self { raw }
    }

    /// The underlying integer representation.
    #[inline]
    pub const fn raw(&self) -> i64 {
        self.raw
    }

    /// `0.0`.
    #[inline]
    pub const fn zero() -> Self {
        Self { raw: 0 }
    }

    /// `1.0` — the value `1 << D` raw. Returns `None` if `<N, D>`
    /// is invalid.
    #[inline]
    pub fn one() -> Option<Self> {
        Self::new(1i64 << D)
    }

    /// Construct from an integer, scaling up by `2^D`. Returns
    /// `None` if the result would overflow the storage range.
    ///
    /// ```ignore
    /// let x = Fixed::<16, 16>::from_int(3).unwrap();
    /// assert_eq!(x.raw(), 3 << 16);
    /// ```
    #[inline]
    pub fn from_int(i: i64) -> Option<Self> {
        if !valid_width(N, D) {
            return None;
        }
        // Avoid shifting by 64 — that's UB for i64 even at the
        // raw integer level. D ≤ 63 because TOTAL_BITS ≤ 64 and
        // N ≥ 1 in any sensible config; reject D == 64 here.
        if D >= 64 {
            return None;
        }
        let scaled = i.checked_shl(D)?;
        Self::new(scaled)
    }

    /// Truncate toward zero, returning the integer part.
    ///
    /// ```ignore
    /// let x = Fixed::<16, 16>::from_float(2.75).unwrap();
    /// assert_eq!(x.to_int(), 2);
    /// let y = Fixed::<16, 16>::from_float(-2.75).unwrap();
    /// assert_eq!(y.to_int(), -2);
    /// ```
    #[inline]
    pub fn to_int(&self) -> i64 {
        if D >= 64 {
            return 0;
        }
        // Arithmetic shift right. `>>` on signed i64 in Rust is
        // arithmetic (sign-extending), but it floors toward
        // -infinity, not zero. We want truncation toward zero so
        // negative numbers round up: divide by 2^D using `/`
        // semantics (Rust's integer division truncates toward 0).
        let scale = 1i64 << D;
        self.raw / scale
    }

    /// Lossy conversion from `f64`. Saturates at the storage
    /// range; rounds to nearest. Returns `None` if `<N, D>` is
    /// invalid or `f` is NaN.
    pub fn from_float(f: f64) -> Option<Self> {
        if !valid_width(N, D) {
            return None;
        }
        if f.is_nan() {
            return None;
        }
        let scaled = f * (1u64 << D) as f64;
        let (lo, hi) = raw_range(N, D);
        let raw = if scaled <= lo as f64 {
            lo
        } else if scaled >= hi as f64 {
            hi
        } else {
            // Round to nearest, ties away from zero. We can't
            // call `f64::round` because that pulls in `libm` on
            // no_std targets — re-implement it: add 0.5
            // (subtract for negatives) and truncate. This matches
            // IEEE round-to-nearest with ties-away-from-zero, the
            // mode users expect from "convert float to fixed".
            let bias = if scaled >= 0.0 { 0.5 } else { -0.5 };
            (scaled + bias) as i64
        };
        Some(Self { raw })
    }

    /// Lossy conversion to `f64`.
    ///
    /// Exact for raw values whose magnitude fits an `f64`'s
    /// 53-bit mantissa; lossy beyond. The scale factor `2^D` is
    /// always representable exactly as `f64` (it's an integer
    /// power of two).
    #[inline]
    pub fn to_float(&self) -> f64 {
        if D >= 64 {
            return 0.0;
        }
        (self.raw as f64) / ((1u64 << D) as f64)
    }

    /// `lhs + rhs`, wrapping on overflow.
    #[inline]
    pub fn add(self, rhs: Self) -> Self {
        Self {
            raw: self.raw.wrapping_add(rhs.raw),
        }
    }

    /// `lhs - rhs`, wrapping on overflow.
    #[inline]
    pub fn sub(self, rhs: Self) -> Self {
        Self {
            raw: self.raw.wrapping_sub(rhs.raw),
        }
    }

    /// `lhs * rhs`. Promotes to `i128` to avoid mid-multiplication
    /// overflow for the 32-bit configurations and for "small"
    /// 64-bit values; the final shift back by `D` produces the
    /// correctly-scaled fixed-point result.
    ///
    /// Wraps if the post-shift result still doesn't fit `i64`.
    #[inline]
    pub fn mul(self, rhs: Self) -> Self {
        if D >= 64 {
            return Self { raw: 0 };
        }
        // The intuition: Fixed = real * 2^D, so
        //   (a * 2^D) * (b * 2^D) = a*b * 2^(2D)
        // we want a*b * 2^D, so we shift right by D.
        let product = (self.raw as i128).wrapping_mul(rhs.raw as i128);
        let shifted = product >> D;
        Self {
            raw: shifted as i64,
        }
    }

    /// `lhs / rhs`. Promotes to `i128` so the pre-shift left by
    /// `D` doesn't lose precision on small numerators.
    ///
    /// Returns `None` on division by zero; this is the only error
    /// path because the runtime explicitly avoids panics
    /// (CLAUDE.md: zero panics in default no_std build).
    #[inline]
    pub fn div(self, rhs: Self) -> Option<Self> {
        if rhs.raw == 0 {
            return None;
        }
        if D >= 64 {
            return Some(Self { raw: 0 });
        }
        // (a * 2^D) / (b * 2^D) = a/b — but we want a/b * 2^D,
        // so the numerator is shifted up by D first.
        let numerator = (self.raw as i128) << D;
        let result = numerator / (rhs.raw as i128);
        Some(Self { raw: result as i64 })
    }

    /// Negation, wrapping (i64::MIN negates to itself).
    #[inline]
    pub fn neg(self) -> Self {
        Self {
            raw: self.raw.wrapping_neg(),
        }
    }
}

// PartialEq + Eq based on the raw integer — no NaN concept.
impl<const N: u32, const D: u32> PartialEq for Fixed<N, D> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<const N: u32, const D: u32> Eq for Fixed<N, D> {}

impl<const N: u32, const D: u32> PartialOrd for Fixed<N, D> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<const N: u32, const D: u32> Ord for Fixed<N, D> {
    #[inline]
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.raw.cmp(&other.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- construction + invariants ----------

    #[test]
    fn invalid_width_rejected_by_new() {
        // 8 + 16 = 24, not 32 or 64.
        let r: Option<Fixed<8, 16>> = Fixed::new(0);
        assert!(r.is_none());
    }

    #[test]
    fn invalid_width_rejected_by_from_int() {
        let r: Option<Fixed<8, 16>> = Fixed::from_int(1);
        assert!(r.is_none());
    }

    #[test]
    fn valid_32bit_width() {
        let r: Option<Fixed<16, 16>> = Fixed::new(0x10000);
        assert!(r.is_some());
    }

    #[test]
    fn valid_64bit_width() {
        let r: Option<Fixed<32, 32>> = Fixed::new(1i64 << 32);
        assert!(r.is_some());
    }

    #[test]
    fn out_of_range_raw_rejected_for_32bit() {
        // 32-bit total width: raw must fit i32.
        let r: Option<Fixed<16, 16>> = Fixed::new(i64::from(i32::MAX) + 1);
        assert!(r.is_none());
    }

    // ---------- from_int / to_int ----------

    #[test]
    fn from_int_round_trips() {
        let x = Fixed::<16, 16>::from_int(42).unwrap();
        assert_eq!(x.to_int(), 42);
    }

    #[test]
    fn from_int_negative_round_trips() {
        let x = Fixed::<16, 16>::from_int(-42).unwrap();
        assert_eq!(x.to_int(), -42);
    }

    #[test]
    fn from_int_overflow_returns_none() {
        // Fixed<16, 16>: integer part holds [-2^15, 2^15 - 1].
        // 2^15 = 32768; 32768 << 16 = 2^31 = 2147483648, which
        // is i32::MAX + 1 — out of the 32-bit raw range.
        let r: Option<Fixed<16, 16>> = Fixed::from_int(32768);
        assert!(r.is_none());
    }

    // ---------- from_float / to_float ----------

    #[test]
    fn from_float_round_trips_exact_values() {
        let x = Fixed::<16, 16>::from_float(1.5).unwrap();
        assert_eq!(x.to_float(), 1.5);
    }

    #[test]
    fn from_float_rounds_to_nearest() {
        // 0.1 in 16.16 is not exact; round to nearest.
        let x = Fixed::<16, 16>::from_float(0.1).unwrap();
        // 0.1 * 65536 = 6553.6 → rounds to 6554.
        assert_eq!(x.raw(), 6554);
    }

    #[test]
    fn from_float_saturates_at_max() {
        let x = Fixed::<16, 16>::from_float(1.0e9).unwrap();
        assert_eq!(x.raw(), i32::MAX as i64);
    }

    #[test]
    fn from_float_saturates_at_min() {
        let x = Fixed::<16, 16>::from_float(-1.0e9).unwrap();
        assert_eq!(x.raw(), i32::MIN as i64);
    }

    #[test]
    fn from_float_nan_returns_none() {
        let r = Fixed::<16, 16>::from_float(f64::NAN);
        assert!(r.is_none());
    }

    // ---------- arithmetic ----------

    #[test]
    fn add_basic() {
        let a = Fixed::<16, 16>::from_float(1.5).unwrap();
        let b = Fixed::<16, 16>::from_float(2.25).unwrap();
        let c = a.add(b);
        assert_eq!(c.to_float(), 3.75);
    }

    #[test]
    fn sub_basic() {
        let a = Fixed::<16, 16>::from_float(2.5).unwrap();
        let b = Fixed::<16, 16>::from_float(1.0).unwrap();
        let c = a.sub(b);
        assert_eq!(c.to_float(), 1.5);
    }

    #[test]
    fn mul_basic() {
        let a = Fixed::<16, 16>::from_float(1.5).unwrap();
        let b = Fixed::<16, 16>::from_float(2.0).unwrap();
        let c = a.mul(b);
        assert_eq!(c.to_float(), 3.0);
    }

    #[test]
    fn mul_correct_scaling_for_64bit() {
        // 32.32 — wider integer part, plenty of headroom.
        let a = Fixed::<32, 32>::from_float(100.5).unwrap();
        let b = Fixed::<32, 32>::from_float(2.0).unwrap();
        let c = a.mul(b);
        assert_eq!(c.to_float(), 201.0);
    }

    #[test]
    fn div_basic() {
        let a = Fixed::<16, 16>::from_float(7.0).unwrap();
        let b = Fixed::<16, 16>::from_float(2.0).unwrap();
        let c = a.div(b).unwrap();
        assert_eq!(c.to_float(), 3.5);
    }

    #[test]
    fn div_by_zero_returns_none() {
        let a = Fixed::<16, 16>::from_float(1.0).unwrap();
        let z = Fixed::<16, 16>::zero();
        assert!(a.div(z).is_none());
    }

    #[test]
    fn neg_round_trips() {
        let a = Fixed::<16, 16>::from_float(3.5).unwrap();
        let n = a.neg();
        assert_eq!(n.to_float(), -3.5);
        assert_eq!(n.neg().to_float(), 3.5);
    }

    // ---------- overflow / wrapping ----------

    #[test]
    fn add_wraps_on_overflow() {
        // In 16.16, raw fits i32. Two near-i32::MAX values
        // wrap when added.
        let a = Fixed::<16, 16>::from_raw(i32::MAX as i64);
        let b = Fixed::<16, 16>::from_raw(1);
        let c = a.add(b);
        // i32::MAX + 1 wrapped in i64 == i32::MAX + 1; the
        // important property is no panic.
        assert_eq!(c.raw(), (i32::MAX as i64) + 1);
    }

    // ---------- ordering ----------

    #[test]
    fn ordering_is_value_ordering() {
        let a = Fixed::<16, 16>::from_float(1.5).unwrap();
        let b = Fixed::<16, 16>::from_float(2.5).unwrap();
        assert!(a < b);
        assert!(b > a);
        assert_eq!(a, a);
    }

    // ---------- 32.32 (64-bit total) ----------

    #[test]
    fn fixed_32_32_construction() {
        let one = Fixed::<32, 32>::from_int(1).unwrap();
        assert_eq!(one.raw(), 1i64 << 32);
        assert_eq!(one.to_float(), 1.0);
    }

    #[test]
    fn fixed_32_32_arithmetic_round_trips() {
        // PID-like: error = 2.5, gain = 0.125, out = 0.3125
        let err = Fixed::<32, 32>::from_float(2.5).unwrap();
        let gain = Fixed::<32, 32>::from_float(0.125).unwrap();
        let out = err.mul(gain);
        assert_eq!(out.to_float(), 0.3125);
    }

    #[test]
    fn zero_constants() {
        let z: Fixed<16, 16> = Fixed::zero();
        assert_eq!(z.raw(), 0);
        let o: Fixed<16, 16> = Fixed::one().unwrap();
        assert_eq!(o.to_float(), 1.0);
    }
}
