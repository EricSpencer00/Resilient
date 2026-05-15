//! RES-2661: Complex number builtins.
//!
//! Complex numbers are represented as `Array<float>` with exactly 2 elements:
//! `[real, imaginary]`. This integrates directly with the existing type system.
//!
//! Construction:
//! * `complex(re, im) -> [float, float]` — create a complex number
//! * `complex_real(z)` — extract real part
//! * `complex_imag(z)` — extract imaginary part
//!
//! Arithmetic:
//! * `complex_add(a, b)` — a + b
//! * `complex_sub(a, b)` — a - b
//! * `complex_mul(a, b)` — a * b
//! * `complex_div(a, b)` — a / b (error if b is zero)
//!
//! Properties:
//! * `complex_abs(z)` — modulus (|z| = sqrt(re² + im²))
//! * `complex_arg(z)` — argument/phase in radians, atan2(im, re)
//! * `complex_conj(z)` — conjugate (re, -im)
//! * `complex_norm_sq(z)` — squared modulus re² + im²
//!
//! Exponential / trigonometric:
//! * `complex_exp(z)` — e^z = e^re * (cos(im) + i*sin(im))
//! * `complex_ln(z)` — natural log ln|z| + i*arg(z)
//! * `complex_pow_real(z, n)` — z^n using De Moivre (n is float)
//! * `complex_sqrt(z)` — principal square root
//! * `complex_sin(z)` — sin(a+bi) = sin(a)cosh(b) + i*cos(a)sinh(b)
//! * `complex_cos(z)` — cos(a+bi) = cos(a)cosh(b) - i*sin(a)sinh(b)

use crate::Value;

type RResult<T> = Result<T, String>;

// ── helpers ───────────────────────────────────────────────────────────────────

fn to_f64(v: &Value, ctx: &str) -> RResult<f64> {
    match v {
        Value::Float(f) => Ok(*f),
        Value::Int(i) => Ok(*i as f64),
        other => Err(format!("{ctx}: expected float or int, got {other}")),
    }
}

/// Extract `(re, im)` from a `[float, float]` complex value.
fn unpack(name: &str, v: &Value) -> RResult<(f64, f64)> {
    match v {
        Value::Array(arr) if arr.len() == 2 => {
            let re = to_f64(&arr[0], &format!("{name}: real part"))?;
            let im = to_f64(&arr[1], &format!("{name}: imaginary part"))?;
            Ok((re, im))
        }
        Value::Array(arr) => Err(format!(
            "{name}: complex number must be Array of length 2, got length {}",
            arr.len()
        )),
        other => Err(format!(
            "{name}: complex number must be [re, im] Array, got {other}"
        )),
    }
}

fn pack(re: f64, im: f64) -> Value {
    Value::Array(vec![Value::Float(re), Value::Float(im)])
}

// ── construction ──────────────────────────────────────────────────────────────

/// `complex(re, im) -> [float, float]`
///
/// Creates a complex number from real and imaginary parts.
///
/// ```text
/// complex(3.0, 4.0)   // == [3.0, 4.0]  (represents 3 + 4i)
/// ```
pub(crate) fn builtin_complex(args: &[Value]) -> RResult<Value> {
    match args {
        [re, im] => {
            let re = to_f64(re, "complex: re")?;
            let im = to_f64(im, "complex: im")?;
            Ok(pack(re, im))
        }
        _ => Err(format!(
            "complex: expected 2 arguments (re, im), got {}",
            args.len()
        )),
    }
}

/// `complex_real(z) -> float`
pub(crate) fn builtin_complex_real(args: &[Value]) -> RResult<Value> {
    match args {
        [z] => {
            let (re, _) = unpack("complex_real", z)?;
            Ok(Value::Float(re))
        }
        _ => Err(format!(
            "complex_real: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `complex_imag(z) -> float`
pub(crate) fn builtin_complex_imag(args: &[Value]) -> RResult<Value> {
    match args {
        [z] => {
            let (_, im) = unpack("complex_imag", z)?;
            Ok(Value::Float(im))
        }
        _ => Err(format!(
            "complex_imag: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ── arithmetic ────────────────────────────────────────────────────────────────

/// `complex_add(a, b) -> [float, float]`
///
/// ```text
/// complex_add([1.0, 2.0], [3.0, 4.0])  // == [4.0, 6.0]
/// ```
pub(crate) fn builtin_complex_add(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b] => {
            let (ar, ai) = unpack("complex_add", a)?;
            let (br, bi) = unpack("complex_add", b)?;
            Ok(pack(ar + br, ai + bi))
        }
        _ => Err(format!(
            "complex_add: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `complex_sub(a, b) -> [float, float]`
pub(crate) fn builtin_complex_sub(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b] => {
            let (ar, ai) = unpack("complex_sub", a)?;
            let (br, bi) = unpack("complex_sub", b)?;
            Ok(pack(ar - br, ai - bi))
        }
        _ => Err(format!(
            "complex_sub: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `complex_mul(a, b) -> [float, float]`
///
/// `(a + bi)(c + di) = (ac - bd) + (ad + bc)i`
pub(crate) fn builtin_complex_mul(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b] => {
            let (ar, ai) = unpack("complex_mul", a)?;
            let (br, bi) = unpack("complex_mul", b)?;
            Ok(pack(ar * br - ai * bi, ar * bi + ai * br))
        }
        _ => Err(format!(
            "complex_mul: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `complex_div(a, b) -> [float, float]`
///
/// `a/b = a * conj(b) / |b|²`
pub(crate) fn builtin_complex_div(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b] => {
            let (ar, ai) = unpack("complex_div", a)?;
            let (br, bi) = unpack("complex_div", b)?;
            let denom = br * br + bi * bi;
            if denom == 0.0 {
                return Err("complex_div: division by zero".to_string());
            }
            Ok(pack(
                (ar * br + ai * bi) / denom,
                (ai * br - ar * bi) / denom,
            ))
        }
        _ => Err(format!(
            "complex_div: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

// ── properties ────────────────────────────────────────────────────────────────

/// `complex_abs(z) -> float`
///
/// Modulus |z| = sqrt(re² + im²).
pub(crate) fn builtin_complex_abs(args: &[Value]) -> RResult<Value> {
    match args {
        [z] => {
            let (re, im) = unpack("complex_abs", z)?;
            Ok(Value::Float((re * re + im * im).sqrt()))
        }
        _ => Err(format!(
            "complex_abs: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `complex_arg(z) -> float`
///
/// Argument (phase) of z in radians: atan2(im, re).
pub(crate) fn builtin_complex_arg(args: &[Value]) -> RResult<Value> {
    match args {
        [z] => {
            let (re, im) = unpack("complex_arg", z)?;
            Ok(Value::Float(im.atan2(re)))
        }
        _ => Err(format!(
            "complex_arg: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `complex_conj(z) -> [float, float]`
///
/// Complex conjugate: (re, -im).
pub(crate) fn builtin_complex_conj(args: &[Value]) -> RResult<Value> {
    match args {
        [z] => {
            let (re, im) = unpack("complex_conj", z)?;
            Ok(pack(re, -im))
        }
        _ => Err(format!(
            "complex_conj: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `complex_norm_sq(z) -> float`
///
/// Squared modulus re² + im². Avoids a sqrt.
pub(crate) fn builtin_complex_norm_sq(args: &[Value]) -> RResult<Value> {
    match args {
        [z] => {
            let (re, im) = unpack("complex_norm_sq", z)?;
            Ok(Value::Float(re * re + im * im))
        }
        _ => Err(format!(
            "complex_norm_sq: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ── exponential / trig ────────────────────────────────────────────────────────

/// `complex_exp(z) -> [float, float]`
///
/// e^(a + bi) = e^a * (cos(b) + i*sin(b))
pub(crate) fn builtin_complex_exp(args: &[Value]) -> RResult<Value> {
    match args {
        [z] => {
            let (re, im) = unpack("complex_exp", z)?;
            let r = re.exp();
            Ok(pack(r * im.cos(), r * im.sin()))
        }
        _ => Err(format!(
            "complex_exp: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `complex_ln(z) -> [float, float]`
///
/// Natural log: ln|z| + i*arg(z). Error if z is zero.
pub(crate) fn builtin_complex_ln(args: &[Value]) -> RResult<Value> {
    match args {
        [z] => {
            let (re, im) = unpack("complex_ln", z)?;
            let r = (re * re + im * im).sqrt();
            if r == 0.0 {
                return Err("complex_ln: logarithm of zero is undefined".to_string());
            }
            Ok(pack(r.ln(), im.atan2(re)))
        }
        _ => Err(format!(
            "complex_ln: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `complex_pow_real(z, n) -> [float, float]`
///
/// Raises z to a real power n via De Moivre's theorem:
/// `|z|^n * (cos(n*arg) + i*sin(n*arg))`.
pub(crate) fn builtin_complex_pow_real(args: &[Value]) -> RResult<Value> {
    match args {
        [z, n_val] => {
            let (re, im) = unpack("complex_pow_real", z)?;
            let n = to_f64(n_val, "complex_pow_real: n")?;
            let r = (re * re + im * im).sqrt();
            let theta = im.atan2(re);
            let rn = r.powf(n);
            Ok(pack(rn * (n * theta).cos(), rn * (n * theta).sin()))
        }
        _ => Err(format!(
            "complex_pow_real: expected 2 arguments (z, n), got {}",
            args.len()
        )),
    }
}

/// `complex_sqrt(z) -> [float, float]`
///
/// Principal square root of z.
pub(crate) fn builtin_complex_sqrt(args: &[Value]) -> RResult<Value> {
    match args {
        [z] => {
            let (re, im) = unpack("complex_sqrt", z)?;
            let r = (re * re + im * im).sqrt();
            let new_r = r.sqrt();
            let theta = im.atan2(re) / 2.0;
            Ok(pack(new_r * theta.cos(), new_r * theta.sin()))
        }
        _ => Err(format!(
            "complex_sqrt: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `complex_sin(z) -> [float, float]`
///
/// sin(a + bi) = sin(a)cosh(b) + i*cos(a)sinh(b)
pub(crate) fn builtin_complex_sin(args: &[Value]) -> RResult<Value> {
    match args {
        [z] => {
            let (a, b) = unpack("complex_sin", z)?;
            Ok(pack(a.sin() * b.cosh(), a.cos() * b.sinh()))
        }
        _ => Err(format!(
            "complex_sin: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `complex_cos(z) -> [float, float]`
///
/// cos(a + bi) = cos(a)cosh(b) - i*sin(a)sinh(b)
pub(crate) fn builtin_complex_cos(args: &[Value]) -> RResult<Value> {
    match args {
        [z] => {
            let (a, b) = unpack("complex_cos", z)?;
            Ok(pack(a.cos() * b.cosh(), -a.sin() * b.sinh()))
        }
        _ => Err(format!(
            "complex_cos: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `complex_from_polar(r, theta) -> [float, float]`
///
/// Create complex number from polar form: r*e^(i*theta).
pub(crate) fn builtin_complex_from_polar(args: &[Value]) -> RResult<Value> {
    match args {
        [r_val, theta_val] => {
            let r = to_f64(r_val, "complex_from_polar: r")?;
            let theta = to_f64(theta_val, "complex_from_polar: theta")?;
            Ok(pack(r * theta.cos(), r * theta.sin()))
        }
        _ => Err(format!(
            "complex_from_polar: expected 2 arguments (r, theta), got {}",
            args.len()
        )),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    fn approx(line: &str, expected: f64) -> bool {
        line.trim()
            .parse::<f64>()
            .map(|v| (v - expected).abs() < 1e-9)
            .unwrap_or(false)
    }

    // ── construction ─────────────────────────────────────────────────────────

    #[test]
    fn complex_create_and_extract() {
        let r = run(r#"let z = complex(3.0, 4.0);
println(complex_real(z));
println(complex_imag(z));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 3.0), "re={}", lines[0]);
        assert!(approx(lines[1], 4.0), "im={}", lines[1]);
    }

    #[test]
    fn complex_from_int_args() {
        let r = run(r#"let z = complex(1, 2);
println(complex_real(z));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, 1.0), "got {line}");
    }

    // ── arithmetic ───────────────────────────────────────────────────────────

    #[test]
    fn complex_add_basic() {
        let r = run(r#"let a = complex(1.0, 2.0);
let b = complex(3.0, 4.0);
let c = complex_add(a, b);
println(complex_real(c));
println(complex_imag(c));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 4.0), "re={}", lines[0]);
        assert!(approx(lines[1], 6.0), "im={}", lines[1]);
    }

    #[test]
    fn complex_sub_basic() {
        let r = run(r#"let a = complex(5.0, 3.0);
let b = complex(2.0, 1.0);
let c = complex_sub(a, b);
println(complex_real(c));
println(complex_imag(c));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 3.0), "re={}", lines[0]);
        assert!(approx(lines[1], 2.0), "im={}", lines[1]);
    }

    #[test]
    fn complex_mul_basic() {
        // (1+2i)(3+4i) = 3+4i+6i+8i² = (3-8)+(4+6)i = -5+10i
        let r = run(r#"let a = complex(1.0, 2.0);
let b = complex(3.0, 4.0);
let c = complex_mul(a, b);
println(complex_real(c));
println(complex_imag(c));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], -5.0), "re={}", lines[0]);
        assert!(approx(lines[1], 10.0), "im={}", lines[1]);
    }

    #[test]
    fn complex_div_basic() {
        // (1+2i)/(1+1i) = (1+2i)(1-i)/2 = (1+2+(-1+2)i)/2 = 3/2 + i/2
        let r = run(r#"let a = complex(1.0, 2.0);
let b = complex(1.0, 1.0);
let c = complex_div(a, b);
println(complex_real(c));
println(complex_imag(c));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 1.5), "re={}", lines[0]);
        assert!(approx(lines[1], 0.5), "im={}", lines[1]);
    }

    #[test]
    fn complex_div_by_zero_errors() {
        let r = run(r#"complex_div(complex(1.0, 1.0), complex(0.0, 0.0));"#);
        assert!(!r.ok, "expected error for division by zero");
    }

    // ── properties ───────────────────────────────────────────────────────────

    #[test]
    fn complex_abs_3_4_is_5() {
        let r = run(r#"println(complex_abs(complex(3.0, 4.0)));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, 5.0), "got {line}");
    }

    #[test]
    fn complex_conj_negates_imag() {
        let r = run(r#"let c = complex_conj(complex(2.0, 3.0));
println(complex_imag(c));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, -3.0), "got {line}");
    }

    #[test]
    fn complex_norm_sq_is_abs_squared() {
        // |3+4i|² = 25
        let r = run(r#"println(complex_norm_sq(complex(3.0, 4.0)));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, 25.0), "got {line}");
    }

    #[test]
    fn complex_arg_pure_imaginary() {
        // arg(i) = π/2
        let r = run(r#"println(complex_arg(complex(0.0, 1.0)));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        let pi_2 = std::f64::consts::PI / 2.0;
        assert!(approx(line, pi_2), "expected π/2={pi_2}, got {line}");
    }

    // ── exponential / trig ───────────────────────────────────────────────────

    #[test]
    fn complex_exp_pure_imaginary_euler() {
        // e^(iπ) = -1 + 0i
        let r = run(r#"let pi = 3.141592653589793;
let z = complex_exp(complex(0.0, pi));
println(complex_real(z));
println(complex_imag(z));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], -1.0), "re={}", lines[0]);
        assert!(approx(lines[1], 0.0), "im={}", lines[1]);
    }

    #[test]
    fn complex_sqrt_of_negative_1() {
        // sqrt(-1) = i
        let r = run(r#"let z = complex_sqrt(complex(0.0 - 1.0, 0.0));
println(complex_real(z));
println(complex_imag(z));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 0.0), "re={}", lines[0]);
        assert!(approx(lines[1], 1.0), "im={}", lines[1]);
    }

    #[test]
    fn complex_ln_of_exp_is_identity() {
        // ln(e^z) ≈ z for z = 1+1i
        let r = run(r#"let z = complex(1.0, 1.0);
let back = complex_ln(complex_exp(z));
println(complex_real(back));
println(complex_imag(back));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 1.0), "re={}", lines[0]);
        assert!(approx(lines[1], 1.0), "im={}", lines[1]);
    }

    #[test]
    fn complex_from_polar_basic() {
        // r=2, theta=π/2 => (0+2i)
        let r = run(r#"let pi = 3.141592653589793;
let z = complex_from_polar(2.0, pi / 2.0);
println(complex_real(z));
println(complex_imag(z));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 0.0), "re={}", lines[0]);
        assert!(approx(lines[1], 2.0), "im={}", lines[1]);
    }

    #[test]
    fn complex_pow_real_square() {
        // (1+i)^2 = 2i
        let r = run(r#"let z = complex_pow_real(complex(1.0, 1.0), 2.0);
println(complex_real(z));
println(complex_imag(z));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 0.0), "re={}", lines[0]);
        assert!(approx(lines[1], 2.0), "im={}", lines[1]);
    }

    // ── sin / cos ────────────────────────────────────────────────────────────

    #[test]
    fn complex_sin_real_axis() {
        // sin(π/2 + 0i) = 1
        let r = run(r#"let pi = 3.141592653589793;
let z = complex_sin(complex(pi / 2.0, 0.0));
println(complex_real(z));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, 1.0), "got {line}");
    }

    #[test]
    fn complex_cos_real_axis() {
        // cos(π + 0i) = -1
        let r = run(r#"let pi = 3.141592653589793;
let z = complex_cos(complex(pi, 0.0));
println(complex_real(z));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, -1.0), "got {line}");
    }

    // ── integration: sin² + cos² = 1 (algebraic identity) ───────────────────

    #[test]
    fn sin_sq_plus_cos_sq_algebraic_one() {
        // sin²(z) + cos²(z) = (1, 0) as a complex number for any z.
        // Test with z = 1+0.5i: compute complex_add(complex_mul(s,s), complex_mul(c,c)).
        let r = run(r#"let z = complex(1.0, 0.5);
let s = complex_sin(z);
let c = complex_cos(z);
let s2 = complex_mul(s, s);
let c2 = complex_mul(c, c);
let sum = complex_add(s2, c2);
println(complex_real(sum));
println(complex_imag(sum));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 1.0), "re(sin²+cos²)={}", lines[0]);
        assert!(approx(lines[1], 0.0), "im(sin²+cos²)={}", lines[1]);
    }
}
