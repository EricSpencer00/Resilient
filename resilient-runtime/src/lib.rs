//! RES-075 Phase A: minimal `#![no_std]` runtime for Resilient.
//!
//! Carves out the value layer + core ops in a separate crate so it
//! can eventually run on a Cortex-M class MCU. Phase A stays
//! alloc-free — it carries `Int(i64)` and `Bool(bool)` only.
//! Float/String/Array/closure variants need allocator support and
//! will land with RES-101 (embedded-alloc) once the no_std boundary
//! is proven.
//!
//! Intentionally NOT pulled into the main `resilient/` crate as a
//! shared dep yet. The two value enums diverge today (the main
//! interpreter's `Value` carries `Box<Node>` for closures, which
//! pulls in alloc transitively); convergence is a follow-up after
//! RES-101 + RES-102 (the embedded example) prove what the
//! embedded surface actually needs.

// `cfg_attr(not(any(test, feature = "std-sink")), no_std)` lets
// the unit-test harness use `std` (the libtest runner needs it),
// and lets the `std-sink` feature pull in `std` for its
// `StdoutSink` impl, while the production lib build stays
// alloc-free for embedded targets when neither applies.
#![cfg_attr(not(any(test, feature = "std-sink")), no_std)]
// The `add`/`sub`/`mul` method names look like std::ops::Add etc.
// to clippy, but our signatures return Result and so can't fit
// the trait shape. Document that the names ARE intentional.
#![allow(clippy::should_implement_trait)]

// RES-178: `static-only` and `alloc` describe incompatible
// runtime postures — the former forbids heap allocation, the
// latter enables it. Fail the build loudly rather than silently
// letting one feature win. Users hitting this should pick one.
#[cfg(all(feature = "alloc", feature = "static-only"))]
compile_error!(
    "`alloc` and `static-only` are mutually exclusive — pick ONE: \
     `alloc` enables heap-bearing Value variants (Value::String today); \
     `static-only` asserts no-heap posture. Both set = ambiguous build intent."
);

// FFI static registry capacity flags are mutually exclusive.
#[cfg(all(feature = "ffi-static-64", feature = "ffi-static-256"))]
compile_error!("`ffi-static-64` and `ffi-static-256` are mutually exclusive.");
#[cfg(all(feature = "ffi-static-64", feature = "ffi-static-1024"))]
compile_error!("`ffi-static-64` and `ffi-static-1024` are mutually exclusive.");
#[cfg(all(feature = "ffi-static-256", feature = "ffi-static-1024"))]
compile_error!("`ffi-static-256` and `ffi-static-1024` are mutually exclusive.");

// RES-098: pull in the `alloc` crate when the `alloc` feature
// is on. Needed in both test and production builds — even when
// std is available, `alloc::string::String` requires the crate
// to be linked.
#[cfg(feature = "alloc")]
extern crate alloc;

// RES-374: heap profiler — peak allocation tracking.
pub mod heap;

// RES-180: `Sink` abstraction + global `print` / `println`
// helpers that route through a user-installed sink.
pub mod live_telemetry;
pub mod sink;

#[cfg(feature = "ffi-static")]
pub mod ffi_static;

#[cfg(feature = "alloc")]
use alloc::string::String;

/// A Resilient runtime value.
///
/// Wrap-on-overflow arithmetic semantics match the bytecode VM
/// (`vm.rs` in the main crate uses `i64::wrapping_add` etc.); this
/// keeps user-visible semantics identical regardless of which
/// backend executes the program.
///
/// Variants:
/// - `Int(i64)` and `Bool(bool)` always available (RES-075 Phase A).
/// - `Float(f64)` always available — no allocator needed for
///   stack-only doubles (RES-098).
/// - `String(alloc::string::String)` only when `--features alloc`
///   is on (RES-098). Embedded users wire a `#[global_allocator]`
///   (e.g. `embedded-alloc::LlffHeap`) at the binary level.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Bool(bool),
    /// RES-098: f64 lives on the stack, no allocator required.
    /// `f64` doesn't impl Eq, so `Value` drops the Eq derive.
    Float(f64),
    /// RES-098: heap-allocated string. Gated on `alloc` feature
    /// because `String` requires `extern crate alloc`.
    #[cfg(feature = "alloc")]
    String(String),
}

/// Errors the runtime can surface from a single op. Mirrors the
/// VM's `VmError` shape so future work can collapse the two if it
/// becomes useful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    /// The op was applied to incompatible Value variants. The
    /// payload names the op so callers can format
    /// `"runtime: type mismatch in {op}"` errors uniformly.
    TypeMismatch(&'static str),
    DivideByZero,
}

impl Value {
    /// `lhs + rhs`.
    /// - `Int + Int`: wrapping i64 add.
    /// - `Float + Float`: f64 add (no overflow concept).
    /// - `String + String` (alloc only): concatenation.
    ///
    /// Mixed-type combinations are a `TypeMismatch` — promotion is
    /// the caller's job.
    pub fn add(self, rhs: Value) -> Result<Value, RuntimeError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_add(b))),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
            #[cfg(feature = "alloc")]
            (Value::String(mut a), Value::String(b)) => {
                a.push_str(&b);
                Ok(Value::String(a))
            }
            _ => Err(RuntimeError::TypeMismatch("add")),
        }
    }

    /// `lhs - rhs`.
    /// - `Int - Int`: wrapping i64 sub.
    /// - `Float - Float`: f64 sub.
    pub fn sub(self, rhs: Value) -> Result<Value, RuntimeError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_sub(b))),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
            _ => Err(RuntimeError::TypeMismatch("sub")),
        }
    }

    /// `lhs * rhs`.
    /// - `Int * Int`: wrapping i64 mul.
    /// - `Float * Float`: f64 mul.
    pub fn mul(self, rhs: Value) -> Result<Value, RuntimeError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_mul(b))),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
            _ => Err(RuntimeError::TypeMismatch("mul")),
        }
    }

    /// `lhs / rhs`.
    /// - `Int / Int`: errors on `rhs == 0`.
    /// - `Float / Float`: produces inf or NaN per IEEE-754, never
    ///   errors. (Matches what the bytecode VM would do once Float
    ///   ops land there.)
    pub fn div(self, rhs: Value) -> Result<Value, RuntimeError> {
        match (self, rhs) {
            (Value::Int(_), Value::Int(0)) => Err(RuntimeError::DivideByZero),
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a / b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
            _ => Err(RuntimeError::TypeMismatch("div")),
        }
    }

    /// `lhs == rhs`. Same-type compares; mixed types are a
    /// TypeMismatch (matches the VM's strict comparison).
    /// Float equality uses bit comparison (`to_bits`) so NaN is
    /// equal to itself — consistent with the constant-pool dedup
    /// in the bytecode VM.
    pub fn eq(self, rhs: Value) -> Result<Value, RuntimeError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a == b)),
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a == b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a.to_bits() == b.to_bits())),
            #[cfg(feature = "alloc")]
            (Value::String(a), Value::String(b)) => Ok(Value::Bool(a == b)),
            _ => Err(RuntimeError::TypeMismatch("eq")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_add_round_trips() {
        let r = Value::Int(2).add(Value::Int(3)).unwrap();
        assert_eq!(r, Value::Int(5));
    }

    #[test]
    fn int_add_wraps_on_overflow() {
        let r = Value::Int(i64::MAX).add(Value::Int(1)).unwrap();
        assert_eq!(r, Value::Int(i64::MIN));
    }

    #[test]
    fn int_sub_and_mul() {
        assert_eq!(Value::Int(10).sub(Value::Int(3)).unwrap(), Value::Int(7));
        assert_eq!(Value::Int(4).mul(Value::Int(5)).unwrap(), Value::Int(20));
    }

    #[test]
    fn int_div_by_zero_is_clean_error() {
        assert_eq!(
            Value::Int(10).div(Value::Int(0)).unwrap_err(),
            RuntimeError::DivideByZero
        );
    }

    #[test]
    fn type_mismatch_on_int_plus_bool() {
        let err = Value::Int(1).add(Value::Bool(true)).unwrap_err();
        assert_eq!(err, RuntimeError::TypeMismatch("add"));
    }

    #[test]
    fn bool_equality_round_trips() {
        assert_eq!(
            Value::Bool(true).eq(Value::Bool(true)).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            Value::Bool(true).eq(Value::Bool(false)).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn mixed_eq_is_type_mismatch() {
        let err = Value::Int(1).eq(Value::Bool(true)).unwrap_err();
        assert_eq!(err, RuntimeError::TypeMismatch("eq"));
    }

    // ---------- RES-098: Float (always available) ----------

    #[test]
    fn float_arithmetic_round_trips() {
        let a = Value::Float(2.5);
        let b = Value::Float(1.5);
        assert_eq!(a.clone().add(b.clone()).unwrap(), Value::Float(4.0));
        assert_eq!(a.clone().sub(b.clone()).unwrap(), Value::Float(1.0));
        assert_eq!(a.clone().mul(b.clone()).unwrap(), Value::Float(3.75));
        assert_eq!(
            Value::Float(10.0).div(Value::Float(4.0)).unwrap(),
            Value::Float(2.5)
        );
    }

    #[test]
    fn float_division_by_zero_yields_inf_not_error() {
        // IEEE-754 div doesn't error — produces inf. Matches the
        // way the bytecode VM will eventually treat float ops.
        let r = Value::Float(1.0).div(Value::Float(0.0)).unwrap();
        match r {
            Value::Float(v) => assert!(v.is_infinite()),
            other => panic!("expected Float(inf), got {:?}", other),
        }
    }

    #[test]
    fn float_eq_uses_bit_compare_so_nan_equals_itself() {
        let nan = Value::Float(f64::NAN);
        let r = nan.clone().eq(nan).unwrap();
        assert_eq!(r, Value::Bool(true));
    }

    #[test]
    fn mixed_int_float_is_type_mismatch() {
        let err = Value::Int(1).add(Value::Float(2.0)).unwrap_err();
        assert_eq!(err, RuntimeError::TypeMismatch("add"));
    }

    // ---------- RES-098: String (gated on `alloc` feature) ----------

    #[cfg(feature = "alloc")]
    #[test]
    fn string_concat() {
        let a = Value::String(String::from("hello, "));
        let b = Value::String(String::from("world"));
        let r = a.add(b).unwrap();
        match r {
            Value::String(s) => assert_eq!(s, "hello, world"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn string_eq() {
        assert_eq!(
            Value::String(String::from("x"))
                .eq(Value::String(String::from("x")))
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            Value::String(String::from("x"))
                .eq(Value::String(String::from("y")))
                .unwrap(),
            Value::Bool(false)
        );
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn string_does_not_subtract() {
        let err = Value::String(String::from("a"))
            .sub(Value::String(String::from("b")))
            .unwrap_err();
        assert_eq!(err, RuntimeError::TypeMismatch("sub"));
    }

    // ---------- RES-178: static-only posture ----------
    //
    // These tests exist to prove the reduced-surface Value still
    // works end-to-end when `alloc` is off — the same assertions
    // the non-feature-gated tests above make, repeated here
    // specifically under `cfg(not(feature = "alloc"))` so
    // `cargo test --features static-only` has coverage that
    // distinguishes "no-alloc runs" from "no-alloc compiles
    // but nothing exercises it". Builders running `cargo test
    // --features static-only` should see these pass.
    //
    // The `#[cfg(all(...))]` pattern is a belt-and-suspenders:
    // `static-only` implies `not alloc` because of the
    // compile_error! at the top of the file, but being explicit
    // about both conditions makes the intent plain.

    #[cfg(all(feature = "static-only", not(feature = "alloc")))]
    #[test]
    fn static_only_int_bool_float_still_work() {
        // Int arithmetic.
        assert_eq!(Value::Int(2).add(Value::Int(3)).unwrap(), Value::Int(5));
        // Bool equality.
        assert_eq!(
            Value::Bool(true).eq(Value::Bool(false)).unwrap(),
            Value::Bool(false)
        );
        // Float arithmetic (f64 lives on the stack — no
        // allocator required).
        assert_eq!(
            Value::Float(1.5).add(Value::Float(2.5)).unwrap(),
            Value::Float(4.0)
        );
    }

    #[cfg(all(feature = "static-only", not(feature = "alloc")))]
    #[test]
    fn static_only_value_enum_omits_string_variant() {
        // Negative assertion by exhaustiveness: a `match` that
        // covers every variant of `Value` compiles under
        // `static-only` with ONLY Int / Bool / Float arms. If
        // the String variant sneaked in (e.g. someone removed
        // its `#[cfg(feature = "alloc")]` gate), this match
        // would fail to compile with a missing-arm error —
        // exactly the regression the ticket wants the build to
        // catch.
        fn is_numeric(v: &Value) -> bool {
            match v {
                Value::Int(_) | Value::Float(_) => true,
                Value::Bool(_) => false,
            }
        }
        assert!(is_numeric(&Value::Int(1)));
        assert!(!is_numeric(&Value::Bool(true)));
        assert!(is_numeric(&Value::Float(1.0)));
    }
}
