//! RES-2654: Combinatorics and discrete collection operations.
//!
//! * `array_cartesian_product(a, b)` — Cartesian product (pairs from two arrays).
//! * `array_combinations(arr, n)` — All n-length subsets (order-independent).
//! * `array_permutations(arr, n)` — All n-length ordered arrangements.
//! * `array_powerset(arr)` — All subsets including the empty set.
//! * `array_transpose(matrix)` — Transpose a 2-D array (array of arrays).
//! * `array_cartesian_product_n(arrays)` — N-way Cartesian product.

use crate::Value;

type RResult<T> = Result<T, String>;

/// `array_cartesian_product(a, b) -> Array`
///
/// Returns an array of all `[x, y]` pairs where `x ∈ a` and `y ∈ b`.
/// The result has `len(a) * len(b)` elements.
///
/// ```text
/// array_cartesian_product([1,2], ["a","b"])
/// // == [[1,"a"], [1,"b"], [2,"a"], [2,"b"]]
/// ```
pub(crate) fn builtin_array_cartesian_product(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(a), Value::Array(b)] => {
            let mut out = Vec::with_capacity(a.len() * b.len());
            for x in a {
                for y in b {
                    out.push(Value::Array(vec![x.clone(), y.clone()]));
                }
            }
            Ok(Value::Array(out))
        }
        [a, _] if !matches!(a, Value::Array(_)) => Err(format!(
            "array_cartesian_product: first argument must be an Array, got {a}"
        )),
        [_, b] if !matches!(b, Value::Array(_)) => Err(format!(
            "array_cartesian_product: second argument must be an Array, got {b}"
        )),
        _ => Err(format!(
            "array_cartesian_product: expected 2 arguments (a, b), got {}",
            args.len()
        )),
    }
}

/// `array_combinations(arr, n) -> Array`
///
/// Returns all length-`n` subsets of `arr` in lexicographic index order.
/// Each subset is itself an array. `n` must be in `0..=len(arr)`.
///
/// ```text
/// array_combinations([1,2,3], 2)
/// // == [[1,2], [1,3], [2,3]]
/// ```
pub(crate) fn builtin_array_combinations(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arr), Value::Int(n)] => {
            let k = *n as usize;
            if *n < 0 {
                return Err(format!("array_combinations: n must be >= 0, got {n}"));
            }
            if k > arr.len() {
                return Err(format!(
                    "array_combinations: n ({n}) exceeds array length ({})",
                    arr.len()
                ));
            }
            // RES-1938: pre-size `out` to the exact C(n, k) when
            // computable. Falls back to Vec::new() on overflow — that
            // case yields gigantic outputs anyway and is dominated by
            // the per-element work below.
            let cap = (0..k).try_fold(1usize, |acc, i| {
                acc.checked_mul(arr.len() - i).map(|m| m / (i + 1))
            });
            let mut out = match cap {
                Some(c) => Vec::with_capacity(c),
                None => Vec::new(),
            };
            let mut indices: Vec<usize> = (0..k).collect();
            if k == 0 {
                out.push(Value::Array(vec![]));
                return Ok(Value::Array(out));
            }
            loop {
                out.push(Value::Array(
                    indices.iter().map(|&i| arr[i].clone()).collect(),
                ));
                // Find the rightmost index that can be incremented.
                let mut i = k;
                while i > 0 && indices[i - 1] == arr.len() - k + i - 1 {
                    i -= 1;
                }
                if i == 0 {
                    break;
                }
                indices[i - 1] += 1;
                for j in i..k {
                    indices[j] = indices[j - 1] + 1;
                }
            }
            Ok(Value::Array(out))
        }
        [Value::Array(_), n] => Err(format!(
            "array_combinations: second argument must be an int, got {n}"
        )),
        [a, _] => Err(format!(
            "array_combinations: first argument must be an Array, got {a}"
        )),
        _ => Err(format!(
            "array_combinations: expected 2 arguments (arr, n), got {}",
            args.len()
        )),
    }
}

/// `array_permutations(arr, n) -> Array`
///
/// Returns all length-`n` ordered arrangements of elements from `arr`.
/// Result has `len(arr)! / (len(arr) - n)!` elements. Elements may repeat
/// positions if the same value appears multiple times in `arr`.
///
/// ```text
/// array_permutations([1,2,3], 2)
/// // == [[1,2],[1,3],[2,1],[2,3],[3,1],[3,2]]
/// ```
pub(crate) fn builtin_array_permutations(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arr), Value::Int(n)] => {
            let k = *n as usize;
            if *n < 0 {
                return Err(format!("array_permutations: n must be >= 0, got {n}"));
            }
            if k > arr.len() {
                return Err(format!(
                    "array_permutations: n ({n}) exceeds array length ({})",
                    arr.len()
                ));
            }
            // RES-1938: pre-size `out` to the exact P(n, k) when
            // computable. Falls back to Vec::new() on overflow — same
            // shape as the combinations pre-size above.
            let cap = (0..k).try_fold(1usize, |acc, i| acc.checked_mul(arr.len() - i));
            let mut out = match cap {
                Some(c) => Vec::with_capacity(c),
                None => Vec::new(),
            };
            let mut used = vec![false; arr.len()];
            let mut current = Vec::with_capacity(k);
            fn permute(
                arr: &[Value],
                k: usize,
                used: &mut Vec<bool>,
                current: &mut Vec<Value>,
                out: &mut Vec<Value>,
            ) {
                if current.len() == k {
                    out.push(Value::Array(current.clone()));
                    return;
                }
                for i in 0..arr.len() {
                    if !used[i] {
                        used[i] = true;
                        current.push(arr[i].clone());
                        permute(arr, k, used, current, out);
                        current.pop();
                        used[i] = false;
                    }
                }
            }
            permute(arr, k, &mut used, &mut current, &mut out);
            Ok(Value::Array(out))
        }
        [Value::Array(_), n] => Err(format!(
            "array_permutations: second argument must be an int, got {n}"
        )),
        [a, _] => Err(format!(
            "array_permutations: first argument must be an Array, got {a}"
        )),
        _ => Err(format!(
            "array_permutations: expected 2 arguments (arr, n), got {}",
            args.len()
        )),
    }
}

/// `array_powerset(arr) -> Array`
///
/// Returns the power set of `arr` — all possible subsets, including the
/// empty set and `arr` itself. The result has `2^len(arr)` elements.
/// Elements are generated in Gray-code order (empty set first).
///
/// ```text
/// array_powerset([1,2])
/// // == [[], [1], [2], [1,2]]
/// ```
pub(crate) fn builtin_array_powerset(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arr)] => {
            let n = arr.len();
            if n > 20 {
                return Err(format!(
                    "array_powerset: array too large ({n} elements) — \
                     result would have 2^{n} subsets"
                ));
            }
            let count = 1usize << n;
            let mut out = Vec::with_capacity(count);
            for mask in 0..count {
                let subset: Vec<Value> = (0..n)
                    .filter(|&i| mask & (1 << i) != 0)
                    .map(|i| arr[i].clone())
                    .collect();
                out.push(Value::Array(subset));
            }
            Ok(Value::Array(out))
        }
        [other] => Err(format!("array_powerset: expected an Array, got {other}")),
        _ => Err(format!(
            "array_powerset: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_transpose(matrix) -> Array`
///
/// Transposes a rectangular 2-D array (array of same-length arrays).
/// Requires all rows to have the same length; errors on empty input or
/// ragged matrices.
///
/// ```text
/// array_transpose([[1,2,3],[4,5,6]])
/// // == [[1,4],[2,5],[3,6]]
/// ```
pub(crate) fn builtin_array_transpose(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(matrix)] => {
            if matrix.is_empty() {
                return Ok(Value::Array(vec![]));
            }
            // Validate all rows are arrays.
            let rows: Vec<&Vec<Value>> = matrix
                .iter()
                .enumerate()
                .map(|(i, row)| match row {
                    Value::Array(r) => Ok(r),
                    other => Err(format!(
                        "array_transpose: row {i} must be an Array, got {other}"
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;

            let ncols = rows[0].len();
            for (i, row) in rows.iter().enumerate() {
                if row.len() != ncols {
                    return Err(format!(
                        "array_transpose: row {i} has {} elements but row 0 has {ncols} \
                         — matrix must be rectangular",
                        row.len()
                    ));
                }
            }

            let mut out = Vec::with_capacity(ncols);
            for col in 0..ncols {
                let new_row: Vec<Value> = rows.iter().map(|row| row[col].clone()).collect();
                out.push(Value::Array(new_row));
            }
            Ok(Value::Array(out))
        }
        [other] => Err(format!(
            "array_transpose: expected an Array of Arrays, got {other}"
        )),
        _ => Err(format!(
            "array_transpose: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_cartesian_product_n(arrays) -> Array`
///
/// N-way Cartesian product of an array of arrays. Each result element is a
/// tuple array. Returns `[[]]` for an empty input (zero-way product).
///
/// ```text
/// array_cartesian_product_n([[1,2],[3,4],[5]])
/// // == [[1,3,5],[1,4,5],[2,3,5],[2,4,5]]
/// ```
pub(crate) fn builtin_array_cartesian_product_n(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arrays)] => {
            // Validate all elements are arrays.
            let arrays_inner: Vec<&Vec<Value>> = arrays
                .iter()
                .enumerate()
                .map(|(i, arr)| match arr {
                    Value::Array(a) => Ok(a),
                    other => Err(format!(
                        "array_cartesian_product_n: element {i} must be an Array, got {other}"
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;

            // Build the product iteratively.
            let mut result: Vec<Vec<Value>> = vec![vec![]];
            for arr in arrays_inner {
                let mut new_result = Vec::with_capacity(result.len() * arr.len());
                for existing in &result {
                    for item in arr {
                        let mut combo = existing.clone();
                        combo.push(item.clone());
                        new_result.push(combo);
                    }
                }
                result = new_result;
            }
            Ok(Value::Array(result.into_iter().map(Value::Array).collect()))
        }
        [other] => Err(format!(
            "array_cartesian_product_n: expected an Array of Arrays, got {other}"
        )),
        _ => Err(format!(
            "array_cartesian_product_n: expected 1 argument, got {}",
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

    // ── array_cartesian_product ───────────────────────────────────────────────

    #[test]
    fn cartesian_product_basic() {
        let r = run(r#"let p = array_cartesian_product([1,2], [10,20]);
println(len(p));
println(p[0][0]);
println(p[0][1]);
println(p[3][0]);
println(p[3][1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "4");
        assert_eq!(lines[1], "1");
        assert_eq!(lines[2], "10");
        assert_eq!(lines[3], "2");
        assert_eq!(lines[4], "20");
    }

    #[test]
    fn cartesian_product_empty_gives_empty() {
        let r = run(r#"let p = array_cartesian_product([], [1,2]);
println(len(p));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    // ── array_combinations ────────────────────────────────────────────────────

    #[test]
    fn combinations_2_of_3() {
        let r = run(r#"let c = array_combinations([1,2,3], 2);
println(len(c));
println(c[0][0]);
println(c[0][1]);
println(c[2][0]);
println(c[2][1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "3");
        assert_eq!(lines[1], "1");
        assert_eq!(lines[2], "2");
        assert_eq!(lines[3], "2");
        assert_eq!(lines[4], "3");
    }

    #[test]
    fn combinations_0_gives_one_empty() {
        let r = run(r#"let c = array_combinations([1,2,3], 0);
println(len(c));
println(len(c[0]));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "1");
        assert_eq!(lines[1], "0");
    }

    #[test]
    fn combinations_all_gives_one() {
        let r = run(r#"let c = array_combinations([1,2,3], 3);
println(len(c));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('1'), "stdout: {}", r.stdout);
    }

    #[test]
    fn combinations_n_exceeds_length_errors() {
        let r = run(r#"let c = array_combinations([1,2], 5);"#);
        assert!(!r.ok, "expected error for n > len");
    }

    // ── array_permutations ────────────────────────────────────────────────────

    #[test]
    fn permutations_2_of_3() {
        let r = run(r#"let p = array_permutations([1,2,3], 2);
println(len(p));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        // 3! / (3-2)! = 6
        assert!(r.stdout.contains('6'), "stdout: {}", r.stdout);
    }

    #[test]
    fn permutations_0_gives_one_empty() {
        let r = run(r#"let p = array_permutations([1,2,3], 0);
println(len(p));
println(len(p[0]));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "1");
        assert_eq!(lines[1], "0");
    }

    // ── array_powerset ────────────────────────────────────────────────────────

    #[test]
    fn powerset_of_2_has_4_elements() {
        let r = run(r#"let ps = array_powerset([1,2]);
println(len(ps));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('4'), "stdout: {}", r.stdout);
    }

    #[test]
    fn powerset_of_empty_has_one_empty_set() {
        let r = run(r#"let ps = array_powerset([]);
println(len(ps));
println(len(ps[0]));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "1");
        assert_eq!(lines[1], "0");
    }

    #[test]
    fn powerset_size_formula() {
        let r = run(r#"let ps = array_powerset([1,2,3]);
println(len(ps));"#);
        // 2^3 = 8
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('8'), "stdout: {}", r.stdout);
    }

    // ── array_transpose ───────────────────────────────────────────────────────

    #[test]
    fn transpose_2x3_gives_3x2() {
        let r = run(r#"let m = [[1,2,3],[4,5,6]];
let t = array_transpose(m);
println(len(t));
println(len(t[0]));
println(t[0][0]);
println(t[0][1]);
println(t[2][1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "3");
        assert_eq!(lines[1], "2");
        assert_eq!(lines[2], "1");
        assert_eq!(lines[3], "4");
        assert_eq!(lines[4], "6");
    }

    #[test]
    fn transpose_empty_gives_empty() {
        let r = run(r#"let t = array_transpose([]);
println(len(t));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    #[test]
    fn transpose_ragged_errors() {
        let r = run(r#"let t = array_transpose([[1,2],[3,4,5]]);"#);
        assert!(!r.ok, "expected error for ragged matrix");
    }

    // ── array_cartesian_product_n ─────────────────────────────────────────────

    #[test]
    fn cartesian_product_n_basic() {
        let r = run(r#"let p = array_cartesian_product_n([[1,2],[3,4]]);
println(len(p));"#);
        // 2 * 2 = 4
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('4'), "stdout: {}", r.stdout);
    }

    #[test]
    fn cartesian_product_n_empty_gives_one_empty() {
        let r = run(r#"let p = array_cartesian_product_n([]);
println(len(p));
println(len(p[0]));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "1");
        assert_eq!(lines[1], "0");
    }

    #[test]
    fn cartesian_product_n_three_arrays() {
        let r = run(r#"let p = array_cartesian_product_n([[0,1],[0,1],[0,1]]);
println(len(p));"#);
        // 2^3 = 8
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('8'), "stdout: {}", r.stdout);
    }
}
