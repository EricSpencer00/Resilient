//! RES-2658: Linear algebra builtins for numeric arrays.
//!
//! All operate on `Array<float>` or `Array<Array<float>>` (matrices).
//! Integers are accepted and promoted to float automatically.
//!
//! Vector operations (1-D arrays):
//! * `vec_add(a, b)` — element-wise addition.
//! * `vec_sub(a, b)` — element-wise subtraction.
//! * `vec_scale(v, s)` — multiply every element by scalar `s`.
//! * `vec_dot(a, b)` — dot (inner) product.
//! * `vec_norm(v)` — Euclidean (L2) norm.
//! * `vec_normalize(v)` — unit vector (v / ||v||).
//! * `vec_cross(a, b)` — 3-D cross product.
//! * `vec_lerp(a, b, t)` — linear interpolation: a + t*(b-a).
//!
//! Matrix operations (2-D arrays, row-major):
//! * `mat_mul(A, B)` — matrix multiplication.
//! * `mat_add(A, B)` — element-wise matrix addition.
//! * `mat_scale(A, s)` — scalar multiply each element.
//! * `mat_transpose(A)` — transpose (see also `array_transpose`).
//! * `mat_identity(n)` — n×n identity matrix.
//! * `mat_trace(A)` — sum of diagonal elements.

use crate::Value;

type RResult<T> = Result<T, String>;

// ── helpers ───────────────────────────────────────────────────────────────────

fn to_float(v: &Value) -> RResult<f64> {
    match v {
        Value::Float(f) => Ok(*f),
        Value::Int(i) => Ok(*i as f64),
        other => Err(format!(
            "linear_algebra: expected numeric element, got {other}"
        )),
    }
}

fn extract_vec(name: &str, v: &Value) -> RResult<Vec<f64>> {
    match v {
        Value::Array(arr) => arr
            .iter()
            .map(to_float)
            .collect::<RResult<Vec<_>>>()
            .map_err(|e| format!("{name}: {e}")),
        other => Err(format!("{name}: expected Array, got {other}")),
    }
}

fn extract_matrix(name: &str, v: &Value) -> RResult<Vec<Vec<f64>>> {
    match v {
        Value::Array(rows) => {
            let mut mat = Vec::with_capacity(rows.len());
            for (i, row) in rows.iter().enumerate() {
                match row {
                    Value::Array(cols) => {
                        let row_f: RResult<Vec<f64>> = cols.iter().map(to_float).collect();
                        mat.push(row_f.map_err(|e| format!("{name}: row {i}: {e}"))?);
                    }
                    other => {
                        return Err(format!(
                            "{name}: expected Array of Arrays, got {other} at row {i}"
                        ));
                    }
                }
            }
            Ok(mat)
        }
        other => Err(format!("{name}: expected Array of Arrays, got {other}")),
    }
}

fn float_array(v: Vec<f64>) -> Value {
    Value::Array(v.into_iter().map(Value::Float).collect())
}

fn float_matrix(m: Vec<Vec<f64>>) -> Value {
    Value::Array(m.into_iter().map(float_array).collect())
}

fn check_same_len(name: &str, a: &[f64], b: &[f64]) -> RResult<()> {
    if a.len() != b.len() {
        Err(format!(
            "{name}: vectors have different lengths ({} vs {})",
            a.len(),
            b.len()
        ))
    } else {
        Ok(())
    }
}

// ── vec_add ───────────────────────────────────────────────────────────────────

/// `vec_add(a, b) -> Array<float>`
///
/// Element-wise sum of two numeric vectors of the same length.
pub(crate) fn builtin_vec_add(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b] => {
            let a = extract_vec("vec_add", a)?;
            let b = extract_vec("vec_add", b)?;
            check_same_len("vec_add", &a, &b)?;
            Ok(float_array(a.iter().zip(&b).map(|(x, y)| x + y).collect()))
        }
        _ => Err(format!("vec_add: expected 2 arguments, got {}", args.len())),
    }
}

// ── vec_sub ───────────────────────────────────────────────────────────────────

/// `vec_sub(a, b) -> Array<float>`
///
/// Element-wise difference `a - b`. Vectors must have the same length.
pub(crate) fn builtin_vec_sub(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b] => {
            let a = extract_vec("vec_sub", a)?;
            let b = extract_vec("vec_sub", b)?;
            check_same_len("vec_sub", &a, &b)?;
            Ok(float_array(a.iter().zip(&b).map(|(x, y)| x - y).collect()))
        }
        _ => Err(format!("vec_sub: expected 2 arguments, got {}", args.len())),
    }
}

// ── vec_scale ─────────────────────────────────────────────────────────────────

/// `vec_scale(v, s) -> Array<float>`
///
/// Multiplies every element of `v` by scalar `s`.
pub(crate) fn builtin_vec_scale(args: &[Value]) -> RResult<Value> {
    match args {
        [v, s] => {
            let v = extract_vec("vec_scale", v)?;
            let s = to_float(s)
                .map_err(|_| format!("vec_scale: second argument must be numeric, got {s}"))?;
            Ok(float_array(v.iter().map(|x| x * s).collect()))
        }
        _ => Err(format!(
            "vec_scale: expected 2 arguments (v, s), got {}",
            args.len()
        )),
    }
}

// ── vec_dot ───────────────────────────────────────────────────────────────────

/// `vec_dot(a, b) -> float`
///
/// Returns the dot (inner) product of two vectors: Σ aᵢ·bᵢ.
pub(crate) fn builtin_vec_dot(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b] => {
            let a = extract_vec("vec_dot", a)?;
            let b = extract_vec("vec_dot", b)?;
            check_same_len("vec_dot", &a, &b)?;
            let dot: f64 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
            Ok(Value::Float(dot))
        }
        _ => Err(format!("vec_dot: expected 2 arguments, got {}", args.len())),
    }
}

// ── vec_norm ──────────────────────────────────────────────────────────────────

/// `vec_norm(v) -> float`
///
/// Returns the Euclidean (L2) norm of `v`: √(Σ xᵢ²).
pub(crate) fn builtin_vec_norm(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let v = extract_vec("vec_norm", v)?;
            let sq_sum: f64 = v.iter().map(|x| x * x).sum();
            Ok(Value::Float(sq_sum.sqrt()))
        }
        _ => Err(format!("vec_norm: expected 1 argument, got {}", args.len())),
    }
}

// ── vec_normalize ─────────────────────────────────────────────────────────────

/// `vec_normalize(v) -> Array<float>`
///
/// Returns the unit vector `v / ||v||`. Errors on the zero vector.
pub(crate) fn builtin_vec_normalize(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let v = extract_vec("vec_normalize", v)?;
            let norm: f64 = v.iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm == 0.0 {
                return Err("vec_normalize: cannot normalize the zero vector".to_string());
            }
            Ok(float_array(v.iter().map(|x| x / norm).collect()))
        }
        _ => Err(format!(
            "vec_normalize: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ── vec_cross ─────────────────────────────────────────────────────────────────

/// `vec_cross(a, b) -> Array<float>`
///
/// Returns the 3-D cross product of two 3-element vectors.
pub(crate) fn builtin_vec_cross(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b] => {
            let a = extract_vec("vec_cross", a)?;
            let b = extract_vec("vec_cross", b)?;
            if a.len() != 3 {
                return Err(format!(
                    "vec_cross: first vector must have 3 elements, got {}",
                    a.len()
                ));
            }
            if b.len() != 3 {
                return Err(format!(
                    "vec_cross: second vector must have 3 elements, got {}",
                    b.len()
                ));
            }
            Ok(float_array(vec![
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]))
        }
        _ => Err(format!(
            "vec_cross: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

// ── vec_lerp ──────────────────────────────────────────────────────────────────

/// `vec_lerp(a, b, t) -> Array<float>`
///
/// Linear interpolation: `a + t * (b - a)`. `t = 0` returns `a`,
/// `t = 1` returns `b`. Vectors must have the same length.
pub(crate) fn builtin_vec_lerp(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b, t] => {
            let a = extract_vec("vec_lerp", a)?;
            let b = extract_vec("vec_lerp", b)?;
            let t = to_float(t).map_err(|_| format!("vec_lerp: t must be numeric, got {t}"))?;
            check_same_len("vec_lerp", &a, &b)?;
            Ok(float_array(
                a.iter()
                    .zip(&b)
                    .map(|(ai, bi)| ai + t * (bi - ai))
                    .collect(),
            ))
        }
        _ => Err(format!(
            "vec_lerp: expected 3 arguments (a, b, t), got {}",
            args.len()
        )),
    }
}

// ── mat_mul ───────────────────────────────────────────────────────────────────

/// `mat_mul(A, B) -> Array<Array<float>>`
///
/// Matrix multiplication. `A` must be m×k and `B` must be k×n;
/// result is m×n.
pub(crate) fn builtin_mat_mul(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b] => {
            let a = extract_matrix("mat_mul", a)?;
            let b = extract_matrix("mat_mul", b)?;
            if a.is_empty() || b.is_empty() {
                return Ok(Value::Array(vec![]));
            }
            let m = a.len();
            let k = a[0].len();
            let n = b[0].len();
            if b.len() != k {
                return Err(format!(
                    "mat_mul: inner dimensions must match — A is {m}×{k} but B has {} rows",
                    b.len()
                ));
            }
            // Check A is rectangular
            for (i, row) in a.iter().enumerate() {
                if row.len() != k {
                    return Err(format!(
                        "mat_mul: A row {i} has {} columns, expected {k}",
                        row.len()
                    ));
                }
            }
            // Check B is rectangular
            for (i, row) in b.iter().enumerate() {
                if row.len() != n {
                    return Err(format!(
                        "mat_mul: B row {i} has {} columns, expected {n}",
                        row.len()
                    ));
                }
            }
            let mut c = vec![vec![0.0f64; n]; m];
            for i in 0..m {
                for j in 0..n {
                    for l in 0..k {
                        c[i][j] += a[i][l] * b[l][j];
                    }
                }
            }
            Ok(float_matrix(c))
        }
        _ => Err(format!("mat_mul: expected 2 arguments, got {}", args.len())),
    }
}

// ── mat_add ───────────────────────────────────────────────────────────────────

/// `mat_add(A, B) -> Array<Array<float>>`
///
/// Element-wise matrix addition. Both matrices must have the same shape.
pub(crate) fn builtin_mat_add(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b] => {
            let a = extract_matrix("mat_add", a)?;
            let b = extract_matrix("mat_add", b)?;
            if a.len() != b.len() {
                return Err(format!(
                    "mat_add: row counts differ ({} vs {})",
                    a.len(),
                    b.len()
                ));
            }
            let result: RResult<Vec<Vec<f64>>> = a
                .iter()
                .zip(&b)
                .enumerate()
                .map(|(i, (ra, rb))| {
                    if ra.len() != rb.len() {
                        Err(format!(
                            "mat_add: row {i} column counts differ ({} vs {})",
                            ra.len(),
                            rb.len()
                        ))
                    } else {
                        Ok(ra.iter().zip(rb).map(|(x, y)| x + y).collect())
                    }
                })
                .collect();
            Ok(float_matrix(result?))
        }
        _ => Err(format!("mat_add: expected 2 arguments, got {}", args.len())),
    }
}

// ── mat_scale ─────────────────────────────────────────────────────────────────

/// `mat_scale(A, s) -> Array<Array<float>>`
///
/// Multiplies every element of matrix `A` by scalar `s`.
pub(crate) fn builtin_mat_scale(args: &[Value]) -> RResult<Value> {
    match args {
        [a, s] => {
            let a = extract_matrix("mat_scale", a)?;
            let s = to_float(s)
                .map_err(|_| format!("mat_scale: second argument must be numeric, got {s}"))?;
            Ok(float_matrix(
                a.iter()
                    .map(|row| row.iter().map(|x| x * s).collect())
                    .collect(),
            ))
        }
        _ => Err(format!(
            "mat_scale: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

// ── mat_transpose ─────────────────────────────────────────────────────────────

/// `mat_transpose(A) -> Array<Array<float>>`
///
/// Transposes matrix `A`. All rows must have the same length.
pub(crate) fn builtin_mat_transpose(args: &[Value]) -> RResult<Value> {
    match args {
        [a] => {
            let a = extract_matrix("mat_transpose", a)?;
            if a.is_empty() {
                return Ok(Value::Array(vec![]));
            }
            let ncols = a[0].len();
            for (i, row) in a.iter().enumerate() {
                if row.len() != ncols {
                    return Err(format!(
                        "mat_transpose: row {i} has {} columns, expected {ncols}",
                        row.len()
                    ));
                }
            }
            let nrows = a.len();
            let transposed: Vec<Vec<f64>> = (0..ncols)
                .map(|j| (0..nrows).map(|i| a[i][j]).collect())
                .collect();
            Ok(float_matrix(transposed))
        }
        _ => Err(format!(
            "mat_transpose: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ── mat_identity ──────────────────────────────────────────────────────────────

/// `mat_identity(n) -> Array<Array<float>>`
///
/// Returns the n×n identity matrix.
pub(crate) fn builtin_mat_identity(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            if *n < 0 {
                return Err(format!("mat_identity: n must be >= 0, got {n}"));
            }
            let n = *n as usize;
            let mat: Vec<Vec<f64>> = (0..n)
                .map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
                .collect();
            Ok(float_matrix(mat))
        }
        [other] => Err(format!("mat_identity: expected int, got {other}")),
        _ => Err(format!(
            "mat_identity: expected 1 argument (n), got {}",
            args.len()
        )),
    }
}

// ── mat_trace ─────────────────────────────────────────────────────────────────

/// `mat_trace(A) -> float`
///
/// Returns the trace of square matrix `A` (sum of diagonal elements).
pub(crate) fn builtin_mat_trace(args: &[Value]) -> RResult<Value> {
    match args {
        [a] => {
            let a = extract_matrix("mat_trace", a)?;
            let n = a.len();
            for (i, row) in a.iter().enumerate() {
                if row.len() != n {
                    return Err(format!(
                        "mat_trace: matrix must be square — row {i} has {} columns, expected {n}",
                        row.len()
                    ));
                }
            }
            let trace: f64 = (0..n).map(|i| a[i][i]).sum();
            Ok(Value::Float(trace))
        }
        _ => Err(format!(
            "mat_trace: expected 1 argument, got {}",
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

    // ── vec_add ───────────────────────────────────────────────────────────────

    #[test]
    fn vec_add_basic() {
        // Floats with fractional part=0 display without ".0" in Resilient
        let r = run("println(vec_add([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('5'), "stdout: {}", r.stdout);
        assert!(r.stdout.contains('7'), "stdout: {}", r.stdout);
        assert!(r.stdout.contains('9'), "stdout: {}", r.stdout);
    }

    #[test]
    fn vec_add_int_inputs() {
        let r = run("println(vec_add([1, 2], [3, 4]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('4'), "stdout: {}", r.stdout);
        assert!(r.stdout.contains('6'), "stdout: {}", r.stdout);
    }

    // ── vec_sub ───────────────────────────────────────────────────────────────

    #[test]
    fn vec_sub_basic() {
        let r = run("println(vec_sub([5.0, 3.0], [2.0, 1.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "stdout: {}", r.stdout);
        assert!(r.stdout.contains('2'), "stdout: {}", r.stdout);
    }

    // ── vec_scale ─────────────────────────────────────────────────────────────

    #[test]
    fn vec_scale_basic() {
        let r = run("println(vec_scale([1.0, 2.0, 3.0], 3.0));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "stdout: {}", r.stdout);
        assert!(r.stdout.contains('6'), "stdout: {}", r.stdout);
        assert!(r.stdout.contains('9'), "stdout: {}", r.stdout);
    }

    // ── vec_dot ───────────────────────────────────────────────────────────────

    #[test]
    fn vec_dot_basic() {
        let r = run("println(vec_dot([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]));");
        // 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("32"), "stdout: {}", r.stdout);
    }

    #[test]
    fn vec_dot_orthogonal() {
        // [1,0] · [0,1] = 0
        let r = run("println(vec_dot([1.0, 0.0], [0.0, 1.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    // ── vec_norm ──────────────────────────────────────────────────────────────

    #[test]
    fn vec_norm_3_4() {
        // |[3, 4]| = 5
        let r = run("println(vec_norm([3.0, 4.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('5'), "stdout: {}", r.stdout);
    }

    #[test]
    fn vec_norm_unit() {
        let r = run("println(vec_norm([1.0, 0.0, 0.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        // norm([1,0,0]) = 1.0; displays as "1" in Resilient
        assert!(r.stdout.trim().starts_with('1'), "stdout: {}", r.stdout);
    }

    // ── vec_normalize ─────────────────────────────────────────────────────────

    #[test]
    fn vec_normalize_basic() {
        let r = run(r#"let v = vec_normalize([3.0, 4.0]);
let norm = vec_norm(v);
println(norm);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        // norm of normalized vector should be ~1.0; parses to float to check
        let val: f64 = r.stdout.trim().parse().unwrap_or(0.0);
        assert!((val - 1.0).abs() < 1e-9, "norm: {}", r.stdout);
    }

    // ── vec_cross ─────────────────────────────────────────────────────────────

    #[test]
    fn vec_cross_basic() {
        // [1,0,0] × [0,1,0] = [0,0,1]; floats display without ".0"
        let r = run("println(vec_cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        // last element should be 1 (the z component)
        assert!(
            r.stdout.contains(", 1]") || r.stdout.contains(", 1\n"),
            "stdout: {}",
            r.stdout
        );
    }

    // ── vec_lerp ──────────────────────────────────────────────────────────────

    #[test]
    fn vec_lerp_midpoint() {
        // lerp([0,0], [10,10], 0.5) = [5,5]
        let r = run("println(vec_lerp([0.0, 0.0], [10.0, 10.0], 0.5));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('5'), "stdout: {}", r.stdout);
    }

    #[test]
    fn vec_lerp_endpoints() {
        let r = run(r#"let a = [1.0, 2.0, 3.0];
let b = [4.0, 5.0, 6.0];
println(vec_lerp(a, b, 0.0));
println(vec_lerp(a, b, 1.0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('1'), "stdout: {}", r.stdout);
        assert!(r.stdout.contains('4'), "stdout: {}", r.stdout);
    }

    // ── mat_mul ───────────────────────────────────────────────────────────────

    fn approx_eq(line: &str, expected: f64) -> bool {
        line.parse::<f64>()
            .map(|v| (v - expected).abs() < 1e-9)
            .unwrap_or(false)
    }

    #[test]
    fn mat_mul_2x2() {
        // [[1,2],[3,4]] × [[5,6],[7,8]] = [[19,22],[43,50]]
        let r = run(r#"let a = [[1.0, 2.0], [3.0, 4.0]];
let b = [[5.0, 6.0], [7.0, 8.0]];
let c = mat_mul(a, b);
println(c[0][0]);
println(c[0][1]);
println(c[1][0]);
println(c[1][1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx_eq(lines[0], 19.0), "c[0][0]: {}", lines[0]);
        assert!(approx_eq(lines[1], 22.0), "c[0][1]: {}", lines[1]);
        assert!(approx_eq(lines[2], 43.0), "c[1][0]: {}", lines[2]);
        assert!(approx_eq(lines[3], 50.0), "c[1][1]: {}", lines[3]);
    }

    #[test]
    fn mat_mul_identity() {
        let r = run(r#"let i = mat_identity(3);
let a = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
let c = mat_mul(i, a);
println(c[0][0]);
println(c[1][1]);
println(c[2][2]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx_eq(lines[0], 1.0), "c[0][0]: {}", lines[0]);
        assert!(approx_eq(lines[1], 5.0), "c[1][1]: {}", lines[1]);
        assert!(approx_eq(lines[2], 9.0), "c[2][2]: {}", lines[2]);
    }

    // ── mat_add ───────────────────────────────────────────────────────────────

    #[test]
    fn mat_add_basic() {
        let r = run(r#"let a = [[1.0, 2.0], [3.0, 4.0]];
let b = [[5.0, 6.0], [7.0, 8.0]];
let c = mat_add(a, b);
println(c[0][0]);
println(c[1][1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx_eq(lines[0], 6.0), "c[0][0]: {}", lines[0]);
        assert!(approx_eq(lines[1], 12.0), "c[1][1]: {}", lines[1]);
    }

    // ── mat_scale ─────────────────────────────────────────────────────────────

    #[test]
    fn mat_scale_basic() {
        let r = run(r#"let a = [[1.0, 2.0], [3.0, 4.0]];
let s = mat_scale(a, 2.0);
println(s[0][0]);
println(s[1][1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx_eq(lines[0], 2.0), "s[0][0]: {}", lines[0]);
        assert!(approx_eq(lines[1], 8.0), "s[1][1]: {}", lines[1]);
    }

    // ── mat_transpose ─────────────────────────────────────────────────────────

    #[test]
    fn mat_transpose_basic() {
        let r = run(r#"let a = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
let t = mat_transpose(a);
println(t[0][0]);
println(t[2][0]);
println(t[0][1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx_eq(lines[0], 1.0), "t[0][0]: {}", lines[0]);
        assert!(approx_eq(lines[1], 3.0), "t[2][0]: {}", lines[1]);
        assert!(approx_eq(lines[2], 4.0), "t[0][1]: {}", lines[2]);
    }

    // ── mat_identity ──────────────────────────────────────────────────────────

    #[test]
    fn mat_identity_3x3() {
        let r = run(r#"let i = mat_identity(3);
println(i[0][0]);
println(i[1][1]);
println(i[2][2]);
println(i[0][1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx_eq(lines[0], 1.0), "i[0][0]: {}", lines[0]);
        assert!(approx_eq(lines[1], 1.0), "i[1][1]: {}", lines[1]);
        assert!(approx_eq(lines[2], 1.0), "i[2][2]: {}", lines[2]);
        assert!(approx_eq(lines[3], 0.0), "i[0][1]: {}", lines[3]);
    }

    // ── mat_trace ─────────────────────────────────────────────────────────────

    #[test]
    fn mat_trace_basic() {
        // trace([[1,2],[3,4]]) = 1 + 4 = 5
        let r = run("println(mat_trace([[1.0, 2.0], [3.0, 4.0]]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('5'), "stdout: {}", r.stdout);
    }

    // ── integration ───────────────────────────────────────────────────────────

    #[test]
    fn cosine_similarity() {
        // cos_sim(a, b) = dot(a,b) / (norm(a) * norm(b))
        let r = run(r#"let a = [1.0, 0.0, 0.0];
let b = [0.0, 1.0, 0.0];
let d = vec_dot(a, b);
let na = vec_norm(a);
let nb = vec_norm(b);
println(d / (na * nb));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    #[test]
    fn projection_via_dot_and_scale() {
        // project a onto unit vector u: (a·u) * u
        let r = run(r#"let a = [3.0, 4.0];
let u = vec_normalize(a);
let magnitude = vec_dot(a, u);
let projected = vec_scale(u, magnitude);
println(projected[0]);
println(projected[1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        // projected should equal a since u is the unit vector along a
        let x: f64 = lines[0].parse().unwrap_or(0.0);
        let y: f64 = lines[1].parse().unwrap_or(0.0);
        assert!((x - 3.0).abs() < 1e-10, "x={x}");
        assert!((y - 4.0).abs() < 1e-10, "y={y}");
    }
}
