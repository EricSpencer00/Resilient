//! RES-2618: `f32` single-precision float type.
//!
//! Cortex-M4F has a hardware single-precision FPU; `f64` operations on
//! that target require software emulation — 4-10× slower and larger.
//! `Float32` is a distinct type from `Float` (`f64`) so the compiler can
//! catch implicit cross-width mixing at the type-check stage.
//!
//! ## What this module provides
//!
//! * `as_f32(x)` builtin — truncates `int` or `float`/`f64` to single
//!   precision. The result is stored as an f64 at runtime (the interpreter
//!   always uses f64 internally) but with f32 precision: rounding and
//!   overflow match IEEE 754-2019 binary32.
//! * `as_f64(x)` builtin — widens `int` or `f32`/`float` to f64.
//! * `check()` — type-consistency pass: flags programs that mix `f32` and
//!   `f64` in arithmetic or assignment without an explicit cast.
//!
//! ## Literal syntax
//!
//! Use `3.14 as f32` or `as_f32(3.14)` for single-precision literals.
//! Full `3.14f32` suffix parsing is deferred to a follow-up PR.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation)]

use crate::{Node, Value};

// ---------------------------------------------------------------------------
// Builtins
// ---------------------------------------------------------------------------

type RResult<T> = Result<T, String>;

/// Convert `int` or `float` (f64) to single-precision float.
/// The value is truncated to the nearest f32 and stored as f64.
pub(crate) fn builtin_as_f32(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Float(*i as f32 as f64)),
        [Value::Float(f)] => Ok(Value::Float(*f as f32 as f64)),
        [other] => Err(format!("as_f32: expected int or float, got {}", other)),
        _ => Err(format!("as_f32: expected 1 argument, got {}", args.len())),
    }
}

/// Convert `int` or `float` (f32 or f64) to double-precision float.
pub(crate) fn builtin_as_f64(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Float(*i as f64)),
        [Value::Float(f)] => Ok(Value::Float(*f)),
        [other] => Err(format!("as_f64: expected int or float, got {}", other)),
        _ => Err(format!("as_f64: expected 1 argument, got {}", args.len())),
    }
}

// ---------------------------------------------------------------------------
// Type-consistency check pass
// ---------------------------------------------------------------------------

/// Typecheck pass for f32/f64 consistency.
///
/// f32 ↔ f64 mixing in arithmetic is already caught by `check_numeric_same_type`
/// in the main typechecker pass, which runs first. This pass is a hook for
/// future f32-specific checks (e.g., f32 annotations on function parameters
/// that accept f64 arguments). For now it is a no-op.
pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn as_f32_truncates_double() {
        let result = builtin_as_f32(&[Value::Float(1.0_f64 / 3.0_f64)]).unwrap();
        let Value::Float(v) = result else {
            panic!("expected Float");
        };
        // 1/3 as f32 = 0.33333334... — the trailing bits beyond f32
        // precision are stripped.
        assert!(
            (v - (1.0_f64 / 3.0_f64) as f32 as f64).abs() < 1e-10,
            "as_f32 should truncate to f32 precision: {v}"
        );
    }

    #[test]
    fn as_f32_from_int() {
        let result = builtin_as_f32(&[Value::Int(42)]).unwrap();
        assert!(matches!(result, Value::Float(v) if (v - 42.0_f64).abs() < 1e-10));
    }

    #[test]
    fn as_f64_from_int() {
        let result = builtin_as_f64(&[Value::Int(100)]).unwrap();
        assert!(matches!(result, Value::Float(v) if (v - 100.0_f64).abs() < 1e-10));
    }

    #[test]
    fn as_f64_from_float() {
        let input = 1.5_f64;
        let result = builtin_as_f64(&[Value::Float(input)]).unwrap();
        assert!(matches!(result, Value::Float(v) if (v - input).abs() < 1e-10));
    }

    #[test]
    fn as_f32_wrong_type_errors() {
        let result = builtin_as_f32(&[Value::Bool(true)]);
        assert!(result.is_err());
    }

    #[test]
    fn as_f64_wrong_arg_count_errors() {
        let result = builtin_as_f64(&[]);
        assert!(result.is_err());
        let result2 = builtin_as_f64(&[Value::Float(1.0), Value::Float(2.0)]);
        assert!(result2.is_err());
    }

    #[test]
    fn f32_type_annotation_accepted() {
        let src = "fn compute(f32 x) -> f32 { return x; }\n";
        let (_prog, errs) = parse(src);
        assert!(
            errs.is_empty(),
            "f32 type annotation should parse cleanly: {errs:?}"
        );
    }

    #[test]
    fn as_f32_cast_in_program() {
        let src = "let x = 3.14 as f32;\nprintln(x);\n";
        let (_prog, errs) = parse(src);
        assert!(errs.is_empty(), "as f32 cast should parse: {errs:?}");
    }

    #[test]
    fn f32_check_pass_is_noop_for_pure_f32() {
        let src = "fn f(f32 x) -> f32 { return x; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
