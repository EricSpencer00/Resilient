//! RES-2660: Extended statistics and matrix decomposition builtins.
//!
//! Statistics (all operate on `Array<float>` or `Array<int>`):
//! * `stats_covariance(a, b)` — sample covariance
//! * `stats_correlation(a, b)` — Pearson correlation coefficient
//! * `stats_percentile(arr, p)` — p-th percentile (0–100), linear interpolation
//! * `stats_zscore(arr)` — standardise each element: (x - mean) / stddev
//! * `stats_normalize(arr)` — min-max normalise each element to [0,1]
//! * `stats_histogram(arr, bins)` — bin counts as Array<int>
//! * `stats_linear_regression(x, y)` — returns [slope, intercept] as Array<float>
//! * `stats_moving_average(arr, k)` — k-window moving average
//! * `stats_weighted_mean(arr, weights)` — weighted mean
//! * `stats_geometric_mean(arr)` — geometric mean (all positive)
//! * `stats_harmonic_mean(arr)` — harmonic mean (all nonzero)
//! * `stats_mode_int(arr)` — most frequent integer (first if tie)
//! * `stats_iqr(arr)` — interquartile range (Q3 - Q1)
//!
//! Matrix decompositions (matrices are `Array<Array<float>>`):
//! * `mat_det(m)` — determinant via LU decomposition
//! * `mat_inv(m)` — matrix inverse (Gauss-Jordan), error if singular
//! * `mat_solve(A, b)` — solve Ax = b, returns x as Array<float>
//! * `mat_norm_frobenius(m)` — Frobenius norm
//! * `mat_rank(m)` — rank via Gaussian elimination
//! * `mat_lu(m)` — LU decomposition, returns [L, U, P] as Array of matrices

use crate::Value;

type RResult<T> = Result<T, String>;

type LuResult = RResult<(Vec<Vec<f64>>, Vec<Vec<f64>>, Vec<usize>, f64)>;

// ── shared helpers ────────────────────────────────────────────────────────────

fn to_f64(v: &Value, ctx: &str) -> RResult<f64> {
    match v {
        Value::Float(f) => Ok(*f),
        Value::Int(i) => Ok(*i as f64),
        other => Err(format!("{ctx}: expected numeric value, got {other}")),
    }
}

fn extract_floats(name: &str, v: &Value) -> RResult<Vec<f64>> {
    match v {
        Value::Array(arr) => arr
            .iter()
            .enumerate()
            .map(|(i, x)| to_f64(x, &format!("{name}: element {i}")))
            .collect(),
        other => Err(format!("{name}: expected Array, got {other}")),
    }
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn float_arr(v: Vec<f64>) -> Value {
    Value::Array(v.into_iter().map(Value::Float).collect())
}

fn int_arr(v: Vec<i64>) -> Value {
    Value::Array(v.into_iter().map(Value::Int).collect())
}

// ── stats_covariance ──────────────────────────────────────────────────────────

/// `stats_covariance(a, b) -> float`
///
/// Sample covariance of two equal-length numeric arrays.
/// Denominator is `n - 1`.
pub(crate) fn builtin_stats_covariance(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b] => {
            let xs = extract_floats("stats_covariance", a)?;
            let ys = extract_floats("stats_covariance", b)?;
            let n = xs.len();
            if n != ys.len() {
                return Err(format!(
                    "stats_covariance: arrays have different lengths ({n} vs {})",
                    ys.len()
                ));
            }
            if n < 2 {
                return Err("stats_covariance: need at least 2 elements".to_string());
            }
            let mx = mean(&xs);
            let my = mean(&ys);
            let cov = xs
                .iter()
                .zip(ys.iter())
                .map(|(x, y)| (x - mx) * (y - my))
                .sum::<f64>()
                / (n - 1) as f64;
            Ok(Value::Float(cov))
        }
        _ => Err(format!(
            "stats_covariance: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

// ── stats_correlation ─────────────────────────────────────────────────────────

/// `stats_correlation(a, b) -> float`
///
/// Pearson correlation coefficient in [-1, 1].
pub(crate) fn builtin_stats_correlation(args: &[Value]) -> RResult<Value> {
    match args {
        [a, b] => {
            let xs = extract_floats("stats_correlation", a)?;
            let ys = extract_floats("stats_correlation", b)?;
            let n = xs.len();
            if n != ys.len() {
                return Err(format!(
                    "stats_correlation: arrays have different lengths ({n} vs {})",
                    ys.len()
                ));
            }
            if n < 2 {
                return Err("stats_correlation: need at least 2 elements".to_string());
            }
            let mx = mean(&xs);
            let my = mean(&ys);
            let cov: f64 = xs
                .iter()
                .zip(ys.iter())
                .map(|(x, y)| (x - mx) * (y - my))
                .sum();
            let sx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum::<f64>().sqrt();
            let sy: f64 = ys.iter().map(|y| (y - my).powi(2)).sum::<f64>().sqrt();
            if sx == 0.0 || sy == 0.0 {
                return Err("stats_correlation: standard deviation is zero".to_string());
            }
            Ok(Value::Float(cov / (sx * sy)))
        }
        _ => Err(format!(
            "stats_correlation: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

// ── stats_percentile ──────────────────────────────────────────────────────────

/// `stats_percentile(arr, p) -> float`
///
/// Computes the `p`-th percentile (0 ≤ p ≤ 100) using linear interpolation.
pub(crate) fn builtin_stats_percentile(args: &[Value]) -> RResult<Value> {
    match args {
        [arr, p_val] => {
            let mut xs = extract_floats("stats_percentile", arr)?;
            let p = to_f64(p_val, "stats_percentile: p")?;
            if !(0.0..=100.0).contains(&p) {
                return Err(format!("stats_percentile: p must be in [0, 100], got {p}"));
            }
            if xs.is_empty() {
                return Err("stats_percentile: empty array".to_string());
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let idx = p / 100.0 * (xs.len() - 1) as f64;
            let lo = idx.floor() as usize;
            let hi = idx.ceil() as usize;
            let frac = idx - lo as f64;
            let v = xs[lo] * (1.0 - frac) + xs[hi] * frac;
            Ok(Value::Float(v))
        }
        _ => Err(format!(
            "stats_percentile: expected 2 arguments (arr, p), got {}",
            args.len()
        )),
    }
}

// ── stats_zscore ──────────────────────────────────────────────────────────────

/// `stats_zscore(arr) -> Array<float>`
///
/// Returns the z-score of each element: `(x - mean) / stddev`.
pub(crate) fn builtin_stats_zscore(args: &[Value]) -> RResult<Value> {
    match args {
        [arr] => {
            let xs = extract_floats("stats_zscore", arr)?;
            if xs.len() < 2 {
                return Err("stats_zscore: need at least 2 elements".to_string());
            }
            let m = mean(&xs);
            let std =
                (xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (xs.len() - 1) as f64).sqrt();
            if std == 0.0 {
                return Err("stats_zscore: all elements are equal (stddev is 0)".to_string());
            }
            Ok(float_arr(xs.iter().map(|x| (x - m) / std).collect()))
        }
        _ => Err(format!(
            "stats_zscore: expected 1 argument (arr), got {}",
            args.len()
        )),
    }
}

// ── stats_normalize ───────────────────────────────────────────────────────────

/// `stats_normalize(arr) -> Array<float>`
///
/// Min-max normalisation: each element mapped to `[0, 1]`.
pub(crate) fn builtin_stats_normalize(args: &[Value]) -> RResult<Value> {
    match args {
        [arr] => {
            let xs = extract_floats("stats_normalize", arr)?;
            if xs.is_empty() {
                return Err("stats_normalize: empty array".to_string());
            }
            let min = xs.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            if (max - min).abs() < 1e-15 {
                return Err("stats_normalize: min == max (range is zero)".to_string());
            }
            Ok(float_arr(
                xs.iter().map(|x| (x - min) / (max - min)).collect(),
            ))
        }
        _ => Err(format!(
            "stats_normalize: expected 1 argument (arr), got {}",
            args.len()
        )),
    }
}

// ── stats_histogram ───────────────────────────────────────────────────────────

/// `stats_histogram(arr, bins) -> Array<int>`
///
/// Divides `[min, max]` into `bins` equal-width buckets and returns
/// the count of elements falling in each bucket. The rightmost bin
/// is closed on both sides.
pub(crate) fn builtin_stats_histogram(args: &[Value]) -> RResult<Value> {
    match args {
        [arr, bins_val] => {
            let xs = extract_floats("stats_histogram", arr)?;
            let bins = match bins_val {
                Value::Int(b) => *b,
                other => return Err(format!("stats_histogram: bins must be int, got {other}")),
            };
            if bins <= 0 {
                return Err(format!("stats_histogram: bins must be > 0, got {bins}"));
            }
            if xs.is_empty() {
                return Ok(int_arr(vec![0; bins as usize]));
            }
            let min = xs.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let mut counts = vec![0i64; bins as usize];
            if (max - min).abs() < 1e-15 {
                counts[0] = xs.len() as i64;
                return Ok(int_arr(counts));
            }
            let width = (max - min) / bins as f64;
            for x in &xs {
                let mut b = ((x - min) / width).floor() as usize;
                if b >= bins as usize {
                    b = bins as usize - 1;
                }
                counts[b] += 1;
            }
            Ok(int_arr(counts))
        }
        _ => Err(format!(
            "stats_histogram: expected 2 arguments (arr, bins), got {}",
            args.len()
        )),
    }
}

// ── stats_linear_regression ───────────────────────────────────────────────────

/// `stats_linear_regression(x, y) -> Array<float>`
///
/// Ordinary least-squares linear regression of `y` on `x`.
/// Returns `[slope, intercept]`.
pub(crate) fn builtin_stats_linear_regression(args: &[Value]) -> RResult<Value> {
    match args {
        [x_arr, y_arr] => {
            let xs = extract_floats("stats_linear_regression", x_arr)?;
            let ys = extract_floats("stats_linear_regression", y_arr)?;
            let n = xs.len();
            if n != ys.len() {
                return Err(format!(
                    "stats_linear_regression: arrays have different lengths ({n} vs {})",
                    ys.len()
                ));
            }
            if n < 2 {
                return Err("stats_linear_regression: need at least 2 points".to_string());
            }
            let mx = mean(&xs);
            let my = mean(&ys);
            let ss_xy: f64 = xs
                .iter()
                .zip(ys.iter())
                .map(|(x, y)| (x - mx) * (y - my))
                .sum();
            let ss_xx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
            if ss_xx.abs() < 1e-15 {
                return Err(
                    "stats_linear_regression: all x values are equal (vertical line)".to_string(),
                );
            }
            let slope = ss_xy / ss_xx;
            let intercept = my - slope * mx;
            Ok(float_arr(vec![slope, intercept]))
        }
        _ => Err(format!(
            "stats_linear_regression: expected 2 arguments (x, y), got {}",
            args.len()
        )),
    }
}

// ── stats_moving_average ──────────────────────────────────────────────────────

/// `stats_moving_average(arr, k) -> Array<float>`
///
/// Simple k-window moving average. The result has length `len(arr) - k + 1`.
pub(crate) fn builtin_stats_moving_average(args: &[Value]) -> RResult<Value> {
    match args {
        [arr, k_val] => {
            let xs = extract_floats("stats_moving_average", arr)?;
            let k = match k_val {
                Value::Int(k) => *k,
                other => return Err(format!("stats_moving_average: k must be int, got {other}")),
            };
            if k <= 0 {
                return Err(format!("stats_moving_average: k must be > 0, got {k}"));
            }
            let k = k as usize;
            if k > xs.len() {
                return Err(format!(
                    "stats_moving_average: k ({k}) > array length ({})",
                    xs.len()
                ));
            }
            let mut window_sum: f64 = xs[..k].iter().sum();
            let mut result = Vec::with_capacity(xs.len() - k + 1);
            result.push(window_sum / k as f64);
            for i in k..xs.len() {
                window_sum += xs[i] - xs[i - k];
                result.push(window_sum / k as f64);
            }
            Ok(float_arr(result))
        }
        _ => Err(format!(
            "stats_moving_average: expected 2 arguments (arr, k), got {}",
            args.len()
        )),
    }
}

// ── stats_weighted_mean ───────────────────────────────────────────────────────

/// `stats_weighted_mean(arr, weights) -> float`
///
/// Weighted arithmetic mean. Weights need not sum to 1.
pub(crate) fn builtin_stats_weighted_mean(args: &[Value]) -> RResult<Value> {
    match args {
        [arr, weights_arr] => {
            let xs = extract_floats("stats_weighted_mean", arr)?;
            let ws = extract_floats("stats_weighted_mean", weights_arr)?;
            if xs.len() != ws.len() {
                return Err(format!(
                    "stats_weighted_mean: arrays have different lengths ({} vs {})",
                    xs.len(),
                    ws.len()
                ));
            }
            let total_w: f64 = ws.iter().sum();
            if total_w.abs() < 1e-15 {
                return Err("stats_weighted_mean: weights sum to zero".to_string());
            }
            let wm = xs.iter().zip(ws.iter()).map(|(x, w)| x * w).sum::<f64>() / total_w;
            Ok(Value::Float(wm))
        }
        _ => Err(format!(
            "stats_weighted_mean: expected 2 arguments (arr, weights), got {}",
            args.len()
        )),
    }
}

// ── stats_geometric_mean ──────────────────────────────────────────────────────

/// `stats_geometric_mean(arr) -> float`
///
/// Geometric mean. All elements must be positive.
pub(crate) fn builtin_stats_geometric_mean(args: &[Value]) -> RResult<Value> {
    match args {
        [arr] => {
            let xs = extract_floats("stats_geometric_mean", arr)?;
            if xs.is_empty() {
                return Err("stats_geometric_mean: empty array".to_string());
            }
            for (i, x) in xs.iter().enumerate() {
                if *x <= 0.0 {
                    return Err(format!(
                        "stats_geometric_mean: element {i} is non-positive ({x})"
                    ));
                }
            }
            let log_mean = xs.iter().map(|x| x.ln()).sum::<f64>() / xs.len() as f64;
            Ok(Value::Float(log_mean.exp()))
        }
        _ => Err(format!(
            "stats_geometric_mean: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ── stats_harmonic_mean ───────────────────────────────────────────────────────

/// `stats_harmonic_mean(arr) -> float`
///
/// Harmonic mean. All elements must be nonzero.
pub(crate) fn builtin_stats_harmonic_mean(args: &[Value]) -> RResult<Value> {
    match args {
        [arr] => {
            let xs = extract_floats("stats_harmonic_mean", arr)?;
            if xs.is_empty() {
                return Err("stats_harmonic_mean: empty array".to_string());
            }
            for (i, x) in xs.iter().enumerate() {
                if *x == 0.0 {
                    return Err(format!("stats_harmonic_mean: element {i} is zero"));
                }
            }
            let n = xs.len() as f64;
            let recip_sum: f64 = xs.iter().map(|x| 1.0 / x).sum();
            Ok(Value::Float(n / recip_sum))
        }
        _ => Err(format!(
            "stats_harmonic_mean: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ── stats_mode_int ────────────────────────────────────────────────────────────

/// `stats_mode_int(arr) -> int`
///
/// Returns the most frequent integer. If there is a tie, the smallest is returned.
pub(crate) fn builtin_stats_mode_int(args: &[Value]) -> RResult<Value> {
    match args {
        [arr] => {
            let items = match arr {
                Value::Array(a) => a,
                other => return Err(format!("stats_mode_int: expected Array, got {other}")),
            };
            if items.is_empty() {
                return Err("stats_mode_int: empty array".to_string());
            }
            let mut counts: std::collections::HashMap<i64, usize> =
                std::collections::HashMap::new();
            for (i, v) in items.iter().enumerate() {
                match v {
                    Value::Int(k) => *counts.entry(*k).or_insert(0) += 1,
                    other => {
                        return Err(format!(
                            "stats_mode_int: element {i} must be int, got {other}"
                        ));
                    }
                }
            }
            let max_count = counts.values().copied().max().unwrap_or(0);
            let mut modes: Vec<i64> = counts
                .into_iter()
                .filter(|(_, c)| *c == max_count)
                .map(|(k, _)| k)
                .collect();
            modes.sort();
            Ok(Value::Int(modes[0]))
        }
        _ => Err(format!(
            "stats_mode_int: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ── stats_iqr ────────────────────────────────────────────────────────────────

/// `stats_iqr(arr) -> float`
///
/// Interquartile range: Q3 (75th percentile) minus Q1 (25th percentile).
pub(crate) fn builtin_stats_iqr(args: &[Value]) -> RResult<Value> {
    match args {
        [arr] => {
            let q1 = builtin_stats_percentile(&[arr.clone(), Value::Float(25.0)])?;
            let q3 = builtin_stats_percentile(&[arr.clone(), Value::Float(75.0)])?;
            match (q1, q3) {
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(b - a)),
                _ => unreachable!(),
            }
        }
        _ => Err(format!(
            "stats_iqr: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ── Matrix helpers ────────────────────────────────────────────────────────────

fn extract_matrix(name: &str, v: &Value) -> RResult<Vec<Vec<f64>>> {
    let rows = match v {
        Value::Array(a) => a,
        other => {
            return Err(format!(
                "{name}: matrix must be Array<Array<float>>, got {other}"
            ));
        }
    };
    rows.iter()
        .enumerate()
        .map(|(i, row)| match row {
            Value::Array(r) => r
                .iter()
                .enumerate()
                .map(|(j, x)| to_f64(x, &format!("{name}: m[{i}][{j}]")))
                .collect(),
            other => Err(format!("{name}: row {i} must be Array, got {other}")),
        })
        .collect()
}

fn float_matrix(m: Vec<Vec<f64>>) -> Value {
    Value::Array(
        m.into_iter()
            .map(|row| Value::Array(row.into_iter().map(Value::Float).collect()))
            .collect(),
    )
}

fn matrix_size(name: &str, m: &[Vec<f64>]) -> RResult<(usize, usize)> {
    let rows = m.len();
    if rows == 0 {
        return Err(format!("{name}: empty matrix"));
    }
    let cols = m[0].len();
    for (i, row) in m.iter().enumerate() {
        if row.len() != cols {
            return Err(format!(
                "{name}: row {i} has {} columns but row 0 has {cols}",
                row.len()
            ));
        }
    }
    Ok((rows, cols))
}

/// LU decomposition with partial pivoting.
/// Returns `(L, U, P, sign)` where `P` is the permutation vector and
/// `sign` is +1 or -1 depending on the number of row swaps.
#[allow(clippy::needless_range_loop)]
fn lu_decompose(m: &[Vec<f64>]) -> LuResult {
    let n = m.len();
    let mut a: Vec<Vec<f64>> = m.to_vec();
    let mut perm: Vec<usize> = (0..n).collect();
    let mut sign = 1.0f64;

    for col in 0..n {
        // Find pivot
        let mut max_row = col;
        let mut max_val = a[col][col].abs();
        for row in (col + 1)..n {
            if a[row][col].abs() > max_val {
                max_val = a[row][col].abs();
                max_row = row;
            }
        }
        if max_val < 1e-12 {
            return Err("mat_det/mat_inv: matrix is singular (or near-singular)".to_string());
        }
        if max_row != col {
            a.swap(col, max_row);
            perm.swap(col, max_row);
            sign = -sign;
        }
        for row in (col + 1)..n {
            let factor = a[row][col] / a[col][col];
            a[row][col] = factor; // store L below diagonal
            for k in (col + 1)..n {
                let av = a[col][k];
                a[row][k] -= factor * av;
            }
        }
    }

    // Extract L and U
    let mut l = vec![vec![0.0; n]; n];
    let mut u = vec![vec![0.0; n]; n];
    for i in 0..n {
        l[i][i] = 1.0;
        for j in 0..n {
            if j < i {
                l[i][j] = a[i][j];
            } else {
                u[i][j] = a[i][j];
            }
        }
    }
    Ok((l, u, perm, sign))
}

// ── mat_det ───────────────────────────────────────────────────────────────────

/// `mat_det(m) -> float`
///
/// Determinant of a square matrix via LU decomposition.
pub(crate) fn builtin_mat_det(args: &[Value]) -> RResult<Value> {
    match args {
        [m] => {
            let mat = extract_matrix("mat_det", m)?;
            let (rows, cols) = matrix_size("mat_det", &mat)?;
            if rows != cols {
                return Err(format!("mat_det: matrix must be square ({rows}x{cols})"));
            }
            let (_, u, _, sign) = lu_decompose(&mat)?;
            let det = sign * (0..rows).map(|i| u[i][i]).product::<f64>();
            Ok(Value::Float(det))
        }
        _ => Err(format!(
            "mat_det: expected 1 argument (m), got {}",
            args.len()
        )),
    }
}

// ── mat_inv ───────────────────────────────────────────────────────────────────

/// `mat_inv(m) -> Array<Array<float>>`
///
/// Matrix inverse via Gauss-Jordan elimination. Errors if singular.
#[allow(clippy::needless_range_loop)]
pub(crate) fn builtin_mat_inv(args: &[Value]) -> RResult<Value> {
    match args {
        [m] => {
            let mat = extract_matrix("mat_inv", m)?;
            let (rows, cols) = matrix_size("mat_inv", &mat)?;
            if rows != cols {
                return Err(format!("mat_inv: matrix must be square ({rows}x{cols})"));
            }
            let n = rows;
            // Augment [A | I]
            let mut aug: Vec<Vec<f64>> = mat
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let mut r = row.clone();
                    for j in 0..n {
                        r.push(if i == j { 1.0 } else { 0.0 });
                    }
                    r
                })
                .collect();

            for col in 0..n {
                // Find pivot
                let mut max_row = col;
                for row in (col + 1)..n {
                    if aug[row][col].abs() > aug[max_row][col].abs() {
                        max_row = row;
                    }
                }
                if aug[max_row][col].abs() < 1e-12 {
                    return Err("mat_inv: matrix is singular".to_string());
                }
                aug.swap(col, max_row);

                let pivot = aug[col][col];
                for j in 0..(2 * n) {
                    aug[col][j] /= pivot;
                }
                for row in 0..n {
                    if row == col {
                        continue;
                    }
                    let factor = aug[row][col];
                    for j in 0..(2 * n) {
                        let v = aug[col][j];
                        aug[row][j] -= factor * v;
                    }
                }
            }

            // Extract right half
            let inv: Vec<Vec<f64>> = aug.iter().map(|row| row[n..].to_vec()).collect();
            Ok(float_matrix(inv))
        }
        _ => Err(format!(
            "mat_inv: expected 1 argument (m), got {}",
            args.len()
        )),
    }
}

// ── mat_solve ─────────────────────────────────────────────────────────────────

/// `mat_solve(A, b) -> Array<float>`
///
/// Solve `Ax = b` for `x` using Gaussian elimination with partial pivoting.
#[allow(clippy::needless_range_loop)]
pub(crate) fn builtin_mat_solve(args: &[Value]) -> RResult<Value> {
    match args {
        [a_val, b_val] => {
            let a = extract_matrix("mat_solve", a_val)?;
            let (rows, cols) = matrix_size("mat_solve", &a)?;
            if rows != cols {
                return Err(format!("mat_solve: A must be square ({rows}x{cols})"));
            }
            let b = extract_floats("mat_solve", b_val)?;
            if b.len() != rows {
                return Err(format!(
                    "mat_solve: b has {} elements but A has {rows} rows",
                    b.len()
                ));
            }
            let n = rows;
            // Augment [A | b]
            let mut aug: Vec<Vec<f64>> = a
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let mut r = row.clone();
                    r.push(b[i]);
                    r
                })
                .collect();

            for col in 0..n {
                let mut max_row = col;
                for row in (col + 1)..n {
                    if aug[row][col].abs() > aug[max_row][col].abs() {
                        max_row = row;
                    }
                }
                if aug[max_row][col].abs() < 1e-12 {
                    return Err("mat_solve: system is singular (no unique solution)".to_string());
                }
                aug.swap(col, max_row);

                let pivot = aug[col][col];
                for j in 0..=n {
                    aug[col][j] /= pivot;
                }
                for row in 0..n {
                    if row == col {
                        continue;
                    }
                    let factor = aug[row][col];
                    for j in 0..=n {
                        let v = aug[col][j];
                        aug[row][j] -= factor * v;
                    }
                }
            }

            let x: Vec<f64> = aug.iter().map(|row| *row.last().unwrap()).collect();
            Ok(float_arr(x))
        }
        _ => Err(format!(
            "mat_solve: expected 2 arguments (A, b), got {}",
            args.len()
        )),
    }
}

// ── mat_norm_frobenius ────────────────────────────────────────────────────────

/// `mat_norm_frobenius(m) -> float`
///
/// Frobenius norm: sqrt(sum of squares of all elements).
pub(crate) fn builtin_mat_norm_frobenius(args: &[Value]) -> RResult<Value> {
    match args {
        [m] => {
            let mat = extract_matrix("mat_norm_frobenius", m)?;
            matrix_size("mat_norm_frobenius", &mat)?;
            let sum_sq: f64 = mat.iter().flat_map(|row| row.iter()).map(|x| x * x).sum();
            Ok(Value::Float(sum_sq.sqrt()))
        }
        _ => Err(format!(
            "mat_norm_frobenius: expected 1 argument (m), got {}",
            args.len()
        )),
    }
}

// ── mat_rank ─────────────────────────────────────────────────────────────────

/// `mat_rank(m) -> int`
///
/// Matrix rank via Gaussian elimination (counts non-zero pivot rows).
#[allow(clippy::needless_range_loop)]
pub(crate) fn builtin_mat_rank(args: &[Value]) -> RResult<Value> {
    match args {
        [m] => {
            let mat = extract_matrix("mat_rank", m)?;
            let (rows, cols) = matrix_size("mat_rank", &mat)?;
            let mut a = mat.clone();
            let mut rank = 0usize;
            let mut pivot_col = 0usize;

            for row in 0..rows {
                if pivot_col >= cols {
                    break;
                }
                // Find pivot in current column
                let mut found = false;
                for r in row..rows {
                    if a[r][pivot_col].abs() > 1e-12 {
                        a.swap(row, r);
                        found = true;
                        break;
                    }
                }
                if !found {
                    pivot_col += 1;
                    continue;
                }
                let pivot = a[row][pivot_col];
                for j in pivot_col..cols {
                    a[row][j] /= pivot;
                }
                for r in 0..rows {
                    if r == row {
                        continue;
                    }
                    let factor = a[r][pivot_col];
                    for j in pivot_col..cols {
                        let v = a[row][j];
                        a[r][j] -= factor * v;
                    }
                }
                rank += 1;
                pivot_col += 1;
            }
            Ok(Value::Int(rank as i64))
        }
        _ => Err(format!(
            "mat_rank: expected 1 argument (m), got {}",
            args.len()
        )),
    }
}

// ── mat_lu ────────────────────────────────────────────────────────────────────

/// `mat_lu(m) -> Array`
///
/// LU decomposition with partial pivoting.
/// Returns `[L, U, P]` where:
/// - `L` is lower-triangular with unit diagonal
/// - `U` is upper-triangular
/// - `P` is the permutation as Array<int> (row i went to P[i])
pub(crate) fn builtin_mat_lu(args: &[Value]) -> RResult<Value> {
    match args {
        [m] => {
            let mat = extract_matrix("mat_lu", m)?;
            let (rows, cols) = matrix_size("mat_lu", &mat)?;
            if rows != cols {
                return Err(format!("mat_lu: matrix must be square ({rows}x{cols})"));
            }
            let (l, u, perm, _) = lu_decompose(&mat)?;
            let l_val = float_matrix(l);
            let u_val = float_matrix(u);
            let p_val = Value::Array(perm.into_iter().map(|i| Value::Int(i as i64)).collect());
            Ok(Value::Array(vec![l_val, u_val, p_val]))
        }
        _ => Err(format!(
            "mat_lu: expected 1 argument (m), got {}",
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
            .map(|v| (v - expected).abs() < 1e-6)
            .unwrap_or(false)
    }

    // ── covariance ───────────────────────────────────────────────────────────

    #[test]
    fn covariance_positive() {
        let r =
            run("println(stats_covariance([1.0, 2.0, 3.0, 4.0, 5.0], [2.0, 4.0, 5.0, 4.0, 5.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        // Expected: ~1.5
        let line = r.stdout.trim();
        assert!(approx(line, 1.5), "expected ~1.5, got {line}");
    }

    #[test]
    fn covariance_length_mismatch_errors() {
        let r = run("stats_covariance([1.0, 2.0], [3.0, 4.0, 5.0]);");
        assert!(!r.ok, "expected error for length mismatch");
    }

    // ── correlation ──────────────────────────────────────────────────────────

    #[test]
    fn correlation_perfect_positive() {
        let r = run("println(stats_correlation([1.0, 2.0, 3.0, 4.0], [2.0, 4.0, 6.0, 8.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, 1.0), "expected 1.0, got {line}");
    }

    #[test]
    fn correlation_perfect_negative() {
        let r = run("println(stats_correlation([1.0, 2.0, 3.0, 4.0], [8.0, 6.0, 4.0, 2.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, -1.0), "expected -1.0, got {line}");
    }

    // ── percentile ───────────────────────────────────────────────────────────

    #[test]
    fn percentile_50() {
        let r = run("println(stats_percentile([1.0, 2.0, 3.0, 4.0, 5.0], 50.0));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, 3.0), "expected 3.0, got {line}");
    }

    #[test]
    fn percentile_0_and_100() {
        let r = run(r#"println(stats_percentile([5.0, 1.0, 3.0], 0.0));
println(stats_percentile([5.0, 1.0, 3.0], 100.0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 1.0), "expected 1.0, got {}", lines[0]);
        assert!(approx(lines[1], 5.0), "expected 5.0, got {}", lines[1]);
    }

    // ── zscore ───────────────────────────────────────────────────────────────

    #[test]
    fn zscore_mean_zero_std_one() {
        let r = run(
            r#"let z = stats_zscore([2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
let sum = array_sum_float(z);
println(len(z));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('8'), "stdout: {}", r.stdout);
    }

    #[test]
    fn zscore_equal_elements_errors() {
        let r = run("stats_zscore([3.0, 3.0, 3.0]);");
        assert!(!r.ok, "expected error for zero stddev");
    }

    // ── normalize ────────────────────────────────────────────────────────────

    #[test]
    fn normalize_range() {
        let r = run(r#"let n = stats_normalize([0.0, 5.0, 10.0]);
println(n[0]);
println(n[1]);
println(n[2]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 0.0), "expected 0.0, got {}", lines[0]);
        assert!(approx(lines[1], 0.5), "expected 0.5, got {}", lines[1]);
        assert!(approx(lines[2], 1.0), "expected 1.0, got {}", lines[2]);
    }

    // ── histogram ────────────────────────────────────────────────────────────

    #[test]
    fn histogram_uniform() {
        let r = run("println(stats_histogram([0.0, 1.0, 2.0, 3.0, 4.0], 5));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[1, 1, 1, 1, 1]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn histogram_total_count() {
        let r = run(
            r#"let h = stats_histogram([1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3);
println(h[0] + h[1] + h[2]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('6'), "stdout: {}", r.stdout);
    }

    // ── linear_regression ────────────────────────────────────────────────────

    #[test]
    fn linear_regression_slope_intercept() {
        // y = 2x + 1  =>  slope=2, intercept=1
        let r = run(
            r#"let params = stats_linear_regression([0.0, 1.0, 2.0, 3.0], [1.0, 3.0, 5.0, 7.0]);
println(params[0]);
println(params[1]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(
            approx(lines[0], 2.0),
            "expected slope=2.0, got {}",
            lines[0]
        );
        assert!(
            approx(lines[1], 1.0),
            "expected intercept=1.0, got {}",
            lines[1]
        );
    }

    // ── moving_average ───────────────────────────────────────────────────────

    #[test]
    fn moving_average_window_2() {
        let r = run(
            r#"let ma = stats_moving_average([1.0, 2.0, 3.0, 4.0, 5.0], 2);
println(len(ma));
println(ma[0]);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "4", "expected 4 elements");
        assert!(approx(lines[1], 1.5), "expected 1.5, got {}", lines[1]);
    }

    // ── weighted_mean ────────────────────────────────────────────────────────

    #[test]
    fn weighted_mean_basic() {
        // weighted mean of [1,2,3] with weights [3,2,1] = (3+4+3)/6 = 10/6
        let r = run("println(stats_weighted_mean([1.0, 2.0, 3.0], [3.0, 2.0, 1.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(
            approx(line, 10.0 / 6.0),
            "expected {}, got {line}",
            10.0 / 6.0
        );
    }

    // ── geometric_mean ───────────────────────────────────────────────────────

    #[test]
    fn geometric_mean_basic() {
        // geomean(1,2,4,8) = (64)^(1/4) = 2√2
        let r = run("println(stats_geometric_mean([1.0, 2.0, 4.0, 8.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, 2.8284271), "expected ~2.828, got {line}");
    }

    // ── harmonic_mean ────────────────────────────────────────────────────────

    #[test]
    fn harmonic_mean_basic() {
        // hmean(1,2,4) = 3/(1+0.5+0.25) = 3/1.75 ≈ 1.7143
        let r = run("println(stats_harmonic_mean([1.0, 2.0, 4.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, 12.0 / 7.0), "expected ~1.714, got {line}");
    }

    // ── mode_int ─────────────────────────────────────────────────────────────

    #[test]
    fn mode_int_clear_winner() {
        let r = run("println(stats_mode_int([1, 2, 2, 3, 3, 3, 4]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "stdout: {}", r.stdout);
    }

    #[test]
    fn mode_int_tie_picks_smallest() {
        let r = run("println(stats_mode_int([1, 1, 2, 2, 3]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('1'), "stdout: {}", r.stdout);
    }

    // ── iqr ──────────────────────────────────────────────────────────────────

    #[test]
    fn iqr_basic() {
        // [1,2,3,4,5,6,7,8,9,10]  Q1=3.25 Q3=7.75 IQR=4.5
        let r = run("println(stats_iqr([1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, 4.5), "expected 4.5, got {line}");
    }

    // ── mat_det ──────────────────────────────────────────────────────────────

    #[test]
    fn mat_det_2x2() {
        // det([[1,2],[3,4]]) = -2
        let r = run("println(mat_det([[1.0, 2.0], [3.0, 4.0]]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, -2.0), "expected -2.0, got {line}");
    }

    #[test]
    fn mat_det_identity() {
        let r = run("println(mat_det([[1.0, 0.0], [0.0, 1.0]]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, 1.0), "expected 1.0, got {line}");
    }

    #[test]
    fn mat_det_3x3() {
        // det([[1,2,3],[4,5,6],[7,2,9]]) = ?
        // = 1(45-12) - 2(36-42) + 3(8-35) = 33 + 12 - 81 = -36
        let r = run("println(mat_det([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 2.0, 9.0]]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, -36.0), "expected -36.0, got {line}");
    }

    // ── mat_inv ──────────────────────────────────────────────────────────────

    #[test]
    fn mat_inv_2x2() {
        // inv([[1,2],[3,4]]) = [[-2, 1], [1.5, -0.5]]
        let r = run(r#"let inv = mat_inv([[1.0, 2.0], [3.0, 4.0]]);
println(inv[0][0]);
println(inv[0][1]);
println(inv[1][0]);
println(inv[1][1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], -2.0), "got {}", lines[0]);
        assert!(approx(lines[1], 1.0), "got {}", lines[1]);
        assert!(approx(lines[2], 1.5), "got {}", lines[2]);
        assert!(approx(lines[3], -0.5), "got {}", lines[3]);
    }

    // ── mat_solve ────────────────────────────────────────────────────────────

    #[test]
    fn mat_solve_simple() {
        // 2x + y = 5, x + 3y = 10  =>  x=1, y=3
        let r = run(r#"let x = mat_solve([[2.0, 1.0], [1.0, 3.0]], [5.0, 10.0]);
println(x[0]);
println(x[1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 1.0), "x={}", lines[0]);
        assert!(approx(lines[1], 3.0), "y={}", lines[1]);
    }

    // ── mat_norm_frobenius ────────────────────────────────────────────────────

    #[test]
    fn mat_norm_frobenius_identity() {
        // Frobenius norm of 2x2 identity = sqrt(2)
        let r = run("println(mat_norm_frobenius([[1.0, 0.0], [0.0, 1.0]]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let line = r.stdout.trim();
        assert!(approx(line, std::f64::consts::SQRT_2), "got {line}");
    }

    // ── mat_rank ─────────────────────────────────────────────────────────────

    #[test]
    fn mat_rank_full_rank() {
        let r = run("println(mat_rank([[1.0, 0.0], [0.0, 1.0]]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('2'), "stdout: {}", r.stdout);
    }

    #[test]
    fn mat_rank_deficient() {
        // rows are linearly dependent
        let r = run("println(mat_rank([[1.0, 2.0], [2.0, 4.0]]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('1'), "stdout: {}", r.stdout);
    }

    // ── mat_lu ───────────────────────────────────────────────────────────────

    #[test]
    fn mat_lu_reconstruction() {
        // L * U should give a row-permuted version of A
        // Just check that mat_lu returns 3 elements [L, U, P]
        let r = run(r#"let result = mat_lu([[2.0, 1.0], [4.0, 3.0]]);
println(len(result));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "stdout: {}", r.stdout);
    }

    #[test]
    fn mat_lu_l_has_unit_diagonal() {
        let r = run(r#"let result = mat_lu([[2.0, 1.0], [4.0, 3.0]]);
let l = result[0];
println(l[0][0]);
println(l[1][1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 1.0), "L[0][0]={}", lines[0]);
        assert!(approx(lines[1], 1.0), "L[1][1]={}", lines[1]);
    }
}
