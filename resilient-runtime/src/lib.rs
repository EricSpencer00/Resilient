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

// `cfg_attr(not(test), no_std)` lets the unit-test harness use
// `std` (the libtest runner needs it), while the production lib
// build stays alloc-free for embedded targets.
#![cfg_attr(not(test), no_std)]
// The `add`/`sub`/`mul` method names look like std::ops::Add etc.
// to clippy, but our signatures return Result and so can't fit
// the trait shape. Document that the names ARE intentional.
#![allow(clippy::should_implement_trait)]

/// A Resilient runtime value. Phase A subset only.
///
/// Wrap-on-overflow arithmetic semantics match the bytecode VM
/// (`vm.rs` in the main crate uses `i64::wrapping_add` etc.); this
/// keeps user-visible semantics identical regardless of which
/// backend executes the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Bool(bool),
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
    /// `lhs + rhs` for two Int values. Wrap-on-overflow.
    pub fn add(self, rhs: Value) -> Result<Value, RuntimeError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_add(b))),
            _ => Err(RuntimeError::TypeMismatch("add")),
        }
    }

    /// `lhs - rhs` for two Int values. Wrap-on-overflow.
    pub fn sub(self, rhs: Value) -> Result<Value, RuntimeError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_sub(b))),
            _ => Err(RuntimeError::TypeMismatch("sub")),
        }
    }

    /// `lhs * rhs` for two Int values. Wrap-on-overflow.
    pub fn mul(self, rhs: Value) -> Result<Value, RuntimeError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_mul(b))),
            _ => Err(RuntimeError::TypeMismatch("mul")),
        }
    }

    /// `lhs / rhs` for two Int values. Returns DivideByZero if
    /// `rhs` is 0.
    pub fn div(self, rhs: Value) -> Result<Value, RuntimeError> {
        match (self, rhs) {
            (Value::Int(_), Value::Int(0)) => Err(RuntimeError::DivideByZero),
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a / b)),
            _ => Err(RuntimeError::TypeMismatch("div")),
        }
    }

    /// `lhs == rhs`. Bool-Bool and Int-Int compare; mixed types
    /// are a TypeMismatch (matches the VM's strict comparison).
    pub fn eq(self, rhs: Value) -> Result<Value, RuntimeError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a == b)),
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a == b)),
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
}
