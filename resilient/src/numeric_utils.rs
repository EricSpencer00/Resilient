//! RES-2650: Numeric utility builtins.
//!
//! * `lerp(a, b, t)` — linear interpolation: `a + t * (b - a)`.
//! * `remap(v, in_lo, in_hi, out_lo, out_hi)` — map `v` from one range to another.
//! * `float_approx_eq(a, b, eps)` — true when `|a - b| <= eps`.
//! * `round_to(x, n)` — round float `x` to `n` decimal places (returns float).
//! * `int_pow(base, exp)` — integer exponentiation; `exp` must be >= 0.

use crate::Value;

type RResult<T> = Result<T, String>;

/// `lerp(a, b, t) -> float`
///
/// Linear interpolation: returns `a + t * (b - a)`. All arguments are
/// promoted to float. `t == 0.0` returns `a`; `t == 1.0` returns `b`.
/// `t` is not clamped, so extrapolation is possible.
///
/// ```text
/// lerp(0.0, 10.0, 0.5)   // == 5.0
/// lerp(0, 100, 0.25)      // == 25.0
/// ```
pub(crate) fn builtin_lerp(args: &[Value]) -> RResult<Value> {
    let to_f = |v: &Value, name: &str| -> RResult<f64> {
        match v {
            Value::Float(f) => Ok(*f),
            Value::Int(n) => Ok(*n as f64),
            other => Err(format!("lerp: {name} must be a number, got {other}")),
        }
    };
    match args {
        [a, b, t] => {
            let fa = to_f(a, "a")?;
            let fb = to_f(b, "b")?;
            let ft = to_f(t, "t")?;
            Ok(Value::Float(fa + ft * (fb - fa)))
        }
        _ => Err(format!(
            "lerp: expected 3 arguments (a, b, t), got {}",
            args.len()
        )),
    }
}

/// `remap(v, in_lo, in_hi, out_lo, out_hi) -> float`
///
/// Maps `v` from the range `[in_lo, in_hi]` to `[out_lo, out_hi]`. All
/// arguments are promoted to float. The output is not clamped. If
/// `in_lo == in_hi` the function returns `out_lo`.
///
/// ```text
/// remap(5.0, 0.0, 10.0, 0.0, 100.0)  // == 50.0
/// remap(0, 0, 255, 0.0, 1.0)          // == 0.0
/// ```
pub(crate) fn builtin_remap(args: &[Value]) -> RResult<Value> {
    let to_f = |v: &Value, name: &str| -> RResult<f64> {
        match v {
            Value::Float(f) => Ok(*f),
            Value::Int(n) => Ok(*n as f64),
            other => Err(format!("remap: {name} must be a number, got {other}")),
        }
    };
    match args {
        [v, in_lo, in_hi, out_lo, out_hi] => {
            let fv = to_f(v, "v")?;
            let fi_lo = to_f(in_lo, "in_lo")?;
            let fi_hi = to_f(in_hi, "in_hi")?;
            let fo_lo = to_f(out_lo, "out_lo")?;
            let fo_hi = to_f(out_hi, "out_hi")?;
            let in_range = fi_hi - fi_lo;
            if in_range == 0.0 {
                return Ok(Value::Float(fo_lo));
            }
            let t = (fv - fi_lo) / in_range;
            Ok(Value::Float(fo_lo + t * (fo_hi - fo_lo)))
        }
        _ => Err(format!(
            "remap: expected 5 arguments (v, in_lo, in_hi, out_lo, out_hi), got {}",
            args.len()
        )),
    }
}

/// `float_approx_eq(a, b, eps) -> bool`
///
/// Returns true when `|a - b| <= eps`. All arguments are promoted to float.
/// Useful for comparing floating-point results where exact equality is
/// unreliable.
///
/// ```text
/// float_approx_eq(0.1 + 0.2, 0.3, 1e-10)  // true
/// ```
pub(crate) fn builtin_float_approx_eq(args: &[Value]) -> RResult<Value> {
    let to_f = |v: &Value, name: &str| -> RResult<f64> {
        match v {
            Value::Float(f) => Ok(*f),
            Value::Int(n) => Ok(*n as f64),
            other => Err(format!(
                "float_approx_eq: {name} must be a number, got {other}"
            )),
        }
    };
    match args {
        [a, b, eps] => {
            let fa = to_f(a, "a")?;
            let fb = to_f(b, "b")?;
            let feps = to_f(eps, "eps")?;
            if feps < 0.0 {
                return Err(format!("float_approx_eq: eps must be >= 0, got {feps}"));
            }
            Ok(Value::Bool((fa - fb).abs() <= feps))
        }
        _ => Err(format!(
            "float_approx_eq: expected 3 arguments (a, b, eps), got {}",
            args.len()
        )),
    }
}

/// `round_to(x, n) -> float`
///
/// Rounds float `x` to `n` decimal places. `n` must be >= 0. Uses
/// round-half-to-even (banker's rounding).
///
/// ```text
/// round_to(3.14159, 2)   // == 3.14
/// round_to(2.5, 0)        // == 2.0  (banker's rounding)
/// ```
pub(crate) fn builtin_round_to(args: &[Value]) -> RResult<Value> {
    let (x, n) = match args {
        [Value::Float(x), Value::Int(n)] => (*x, *n),
        [Value::Int(x), Value::Int(n)] => (*x as f64, *n),
        [_, Value::Int(_)] => {
            return Err(format!(
                "round_to: first argument must be a number, got {}",
                args[0]
            ));
        }
        [_, n] => return Err(format!("round_to: second argument must be an int, got {n}")),
        _ => {
            return Err(format!(
                "round_to: expected 2 arguments (x, n), got {}",
                args.len()
            ));
        }
    };
    if n < 0 {
        return Err(format!("round_to: decimal places must be >= 0, got {n}"));
    }
    if n > 308 {
        return Err(format!(
            "round_to: decimal places must be <= 308 (f64 precision limit), got {n}"
        ));
    }
    let factor = 10f64.powi(n as i32);
    Ok(Value::Float((x * factor).round() / factor))
}

/// `int_pow(base, exp) -> int`
///
/// Computes `base ^ exp` using integer arithmetic. `exp` must be >= 0.
/// Wraps on overflow (saturating semantics not applied, consistent with
/// other integer operations in Resilient).
///
/// ```text
/// int_pow(2, 10)   // == 1024
/// int_pow(3, 0)    // == 1
/// ```
pub(crate) fn builtin_int_pow(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(base), Value::Int(exp)] => {
            if *exp < 0 {
                return Err(format!("int_pow: exponent must be >= 0, got {exp}"));
            }
            Ok(Value::Int(base.wrapping_pow(*exp as u32)))
        }
        [Value::Int(_), e] => Err(format!("int_pow: exponent must be an int, got {e}")),
        [b, _] => Err(format!("int_pow: base must be an int, got {b}")),
        _ => Err(format!(
            "int_pow: expected 2 arguments (base, exp), got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── lerp ──────────────────────────────────────────────────────────────────

    #[test]
    fn lerp_midpoint() {
        let r = run(r#"println(lerp(0.0, 10.0, 0.5));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('5'), "stdout: {}", r.stdout);
    }

    #[test]
    fn lerp_endpoints() {
        let r = run(r#"println(lerp(3.0, 7.0, 0.0));
println(lerp(3.0, 7.0, 1.0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(lines[0].starts_with('3'), "t=0 gives a: {}", lines[0]);
        assert!(lines[1].starts_with('7'), "t=1 gives b: {}", lines[1]);
    }

    #[test]
    fn lerp_int_args_promoted() {
        let r = run(r#"println(lerp(0, 100, 0.25));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("25"), "stdout: {}", r.stdout);
    }

    // ── remap ─────────────────────────────────────────────────────────────────

    #[test]
    fn remap_midpoint() {
        let r = run(r#"println(remap(5.0, 0.0, 10.0, 0.0, 100.0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("50"), "stdout: {}", r.stdout);
    }

    #[test]
    fn remap_zero_in_range_returns_out_lo() {
        let r = run(r#"println(remap(5.0, 5.0, 5.0, 100.0, 200.0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("100"), "stdout: {}", r.stdout);
    }

    #[test]
    fn remap_int_to_float_range() {
        let r = run(r#"println(remap(128, 0, 255, 0.0, 1.0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.starts_with('0'), "stdout: {}", r.stdout);
    }

    // ── float_approx_eq ───────────────────────────────────────────────────────

    #[test]
    fn approx_eq_close_values() {
        let r = run(r#"println(float_approx_eq(1.0, 1.0000000001, 1e-5));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[test]
    fn approx_eq_distant_values() {
        let r = run(r#"println(float_approx_eq(1.0, 2.0, 0.5));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("false"), "stdout: {}", r.stdout);
    }

    #[test]
    fn approx_eq_exact_same() {
        let r = run(r#"println(float_approx_eq(42.0, 42.0, 0.0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    // ── round_to ──────────────────────────────────────────────────────────────

    #[test]
    fn round_to_two_places() {
        let r = run(r#"println(round_to(3.14159, 2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("3.14"), "stdout: {}", r.stdout);
    }

    #[test]
    fn round_to_zero_places() {
        let r = run(r#"println(round_to(2.7, 0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "stdout: {}", r.stdout);
    }

    #[test]
    fn round_to_negative_n_errors() {
        let r = run(r#"println(round_to(3.14, -1));"#);
        assert!(!r.ok, "expected error for negative n");
    }

    // ── int_pow ───────────────────────────────────────────────────────────────

    #[test]
    fn int_pow_basic() {
        let r = run(r#"println(int_pow(2, 10));
println(int_pow(3, 0));
println(int_pow(5, 3));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "1024");
        assert_eq!(lines[1], "1");
        assert_eq!(lines[2], "125");
    }

    #[test]
    fn int_pow_negative_exp_errors() {
        let r = run(r#"println(int_pow(2, -1));"#);
        assert!(!r.ok, "expected error for negative exponent");
    }
}
