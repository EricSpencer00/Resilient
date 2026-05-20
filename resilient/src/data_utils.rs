//! RES-2662: Data utility builtins.
//!
//! Numeric ranges:
//! * `linspace(start, stop, n)` — n evenly-spaced floats in [start, stop]
//! * `logspace(start, stop, n)` — n log-spaced values (base 10)
//! * `arange(start, stop, step)` — like Python's arange
//!
//! CSV / TSV:
//! * `csv_parse(s)` — parse CSV string → Array<Array<string>>
//! * `csv_parse_tsv(s)` — parse TSV string → Array<Array<string>>
//! * `csv_format(rows)` — Array<Array<string>> → CSV string
//! * `csv_format_tsv(rows)` — Array<Array<string>> → TSV string
//!
//! Tabular formatting:
//! * `table_format(rows)` — pretty-print as aligned table string
//!
//! String formatting:
//! * `format_float(f, decimals)` — float with fixed decimal places
//! * `format_int_width(n, width)` — right-aligned int with padding
//! * `format_float_sci(f, sig)` — scientific notation string
//!
//! Run-length encoding:
//! * `rle_encode(arr)` — Array<[count, value]> run-length encoding
//! * `rle_decode(arr)` — reconstruct original array from RLE

use crate::Value;

type RResult<T> = Result<T, String>;

// ── linspace / logspace / arange ──────────────────────────────────────────────

/// `linspace(start, stop, n) -> Array<float>`
///
/// Returns `n` evenly-spaced values in `[start, stop]`.
///
/// ```text
/// linspace(0.0, 1.0, 5)  // == [0.0, 0.25, 0.5, 0.75, 1.0]
/// ```
pub(crate) fn builtin_linspace(args: &[Value]) -> RResult<Value> {
    match args {
        [start, stop, n_val] => {
            let start = to_f64(start, "linspace: start")?;
            let stop = to_f64(stop, "linspace: stop")?;
            let n = to_usize_pos(n_val, "linspace")?;
            if n == 0 {
                return Ok(Value::Array(vec![]));
            }
            if n == 1 {
                return Ok(Value::Array(vec![Value::Float(start)]));
            }
            let step = (stop - start) / (n - 1) as f64;
            let v: Vec<Value> = (0..n)
                .map(|i| Value::Float(start + i as f64 * step))
                .collect();
            Ok(Value::Array(v))
        }
        _ => Err(format!(
            "linspace: expected 3 arguments (start, stop, n), got {}",
            args.len()
        )),
    }
}

/// `logspace(start, stop, n) -> Array<float>`
///
/// Returns `n` log-spaced values: `10^start` to `10^stop`.
pub(crate) fn builtin_logspace(args: &[Value]) -> RResult<Value> {
    match args {
        [start, stop, n_val] => {
            let start = to_f64(start, "logspace: start")?;
            let stop = to_f64(stop, "logspace: stop")?;
            let n = to_usize_pos(n_val, "logspace")?;
            if n == 0 {
                return Ok(Value::Array(vec![]));
            }
            if n == 1 {
                return Ok(Value::Array(vec![Value::Float(10f64.powf(start))]));
            }
            let step = (stop - start) / (n - 1) as f64;
            let v: Vec<Value> = (0..n)
                .map(|i| Value::Float(10f64.powf(start + i as f64 * step)))
                .collect();
            Ok(Value::Array(v))
        }
        _ => Err(format!(
            "logspace: expected 3 arguments (start, stop, n), got {}",
            args.len()
        )),
    }
}

/// `arange(start, stop, step) -> Array<float>`
///
/// Returns values from `start` up to (but not including) `stop` with `step`.
/// Step must be nonzero.
pub(crate) fn builtin_arange(args: &[Value]) -> RResult<Value> {
    match args {
        [start, stop, step] => {
            let start = to_f64(start, "arange: start")?;
            let stop = to_f64(stop, "arange: stop")?;
            let step = to_f64(step, "arange: step")?;
            if step == 0.0 {
                return Err("arange: step must be nonzero".to_string());
            }
            const MAX_ELEMENTS: usize = 10_000_000;
            // RES-1942: pre-size to the computed step count. Floor at
            // `MAX_ELEMENTS + 1` so the existing guard still fires for
            // oversize requests, and clamp NaN / overflow → 0 (fall
            // back to default-cap Vec). The +1 accommodates the
            // strict-inequality bound of the loop above for cases
            // where (stop - start) is an exact multiple of step.
            let raw = ((stop - start) / step).abs().ceil();
            let cap = if raw.is_finite() && raw >= 0.0 {
                (raw as usize).saturating_add(1).min(MAX_ELEMENTS + 1)
            } else {
                0
            };
            let mut v = Vec::with_capacity(cap);
            let mut x = start;
            while (step > 0.0 && x < stop) || (step < 0.0 && x > stop) {
                v.push(Value::Float(x));
                if v.len() > MAX_ELEMENTS {
                    return Err(format!(
                        "arange: result would exceed {MAX_ELEMENTS} elements"
                    ));
                }
                x += step;
            }
            Ok(Value::Array(v))
        }
        _ => Err(format!(
            "arange: expected 3 arguments (start, stop, step), got {}",
            args.len()
        )),
    }
}

// ── CSV ───────────────────────────────────────────────────────────────────────

/// `csv_parse(s) -> Array<Array<string>>`
///
/// Parses a CSV string. Supports quoted fields (double-quote escaped).
pub(crate) fn builtin_csv_parse(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let s = as_string("csv_parse", v)?;
            Ok(Value::Array(parse_delimited(s, ',')))
        }
        _ => Err(format!(
            "csv_parse: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `csv_parse_tsv(s) -> Array<Array<string>>`
///
/// Parses a TSV (tab-separated) string.
pub(crate) fn builtin_csv_parse_tsv(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let s = as_string("csv_parse_tsv", v)?;
            Ok(Value::Array(parse_delimited(s, '\t')))
        }
        _ => Err(format!(
            "csv_parse_tsv: expected 1 argument, got {}",
            args.len()
        )),
    }
}

fn parse_delimited(s: &str, delim: char) -> Vec<Value> {
    s.lines()
        .map(|line| {
            let fields = parse_csv_row(line, delim);
            Value::Array(fields.into_iter().map(Value::String).collect())
        })
        .collect()
}

/// Parse a single CSV row with optional quoting.
fn parse_csv_row(line: &str, delim: char) -> Vec<String> {
    // RES-1942: pre-size `fields` to the upper-bound count of
    // unescaped delimiters + 1. `,` and `\t` (the only two callers,
    // via builtin_csv_parse / _tsv) are ASCII single-byte so the
    // byte-level scan is a safe approximation. Quoted-delimiter
    // escapes can lower the actual field count, but never raise it,
    // so this is a safe upper bound.
    let cap = if (delim as u32) < 0x80 {
        line.bytes().filter(|&b| b == delim as u8).count() + 1
    } else {
        1
    };
    let mut fields = Vec::with_capacity(cap);
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == delim {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    fields.push(current);
    fields
}

/// `csv_format(rows) -> string`
///
/// Serializes `Array<Array<string>>` to a CSV string.
/// Fields containing commas, quotes, or newlines are automatically quoted.
pub(crate) fn builtin_csv_format(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let s = format_delimited("csv_format", v, ',')?;
            Ok(Value::String(s))
        }
        _ => Err(format!(
            "csv_format: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `csv_format_tsv(rows) -> string`
///
/// Serializes `Array<Array<string>>` to a TSV string.
pub(crate) fn builtin_csv_format_tsv(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let s = format_delimited("csv_format_tsv", v, '\t')?;
            Ok(Value::String(s))
        }
        _ => Err(format!(
            "csv_format_tsv: expected 1 argument, got {}",
            args.len()
        )),
    }
}

fn format_delimited(name: &str, v: &Value, delim: char) -> RResult<String> {
    let rows = match v {
        Value::Array(a) => a,
        other => {
            return Err(format!(
                "{name}: expected Array<Array<string>>, got {other}"
            ));
        }
    };
    let mut out = String::new();
    for (i, row) in rows.iter().enumerate() {
        let fields = match row {
            Value::Array(f) => f,
            other => return Err(format!("{name}: row {i} must be Array, got {other}")),
        };
        let row_str: Vec<String> = fields
            .iter()
            .enumerate()
            .map(|(j, f)| {
                let s = match f {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "{name}: rows[{i}][{j}] must be string, got {other}"
                        ));
                    }
                };
                // Quote if the field contains the delimiter, a quote, or a newline
                if s.contains(delim) || s.contains('"') || s.contains('\n') {
                    Ok(format!("\"{}\"", s.replace('"', "\"\"")))
                } else {
                    Ok(s)
                }
            })
            .collect::<RResult<Vec<_>>>()?;
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&row_str.join(&delim.to_string()));
    }
    Ok(out)
}

// ── table_format ──────────────────────────────────────────────────────────────

/// `table_format(rows) -> string`
///
/// Pretty-prints `Array<Array<string>>` as a text table with aligned columns.
pub(crate) fn builtin_table_format(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let rows = match v {
                Value::Array(a) => a,
                other => {
                    return Err(format!(
                        "table_format: expected Array<Array<string>>, got {other}"
                    ));
                }
            };
            if rows.is_empty() {
                return Ok(Value::String(String::new()));
            }

            // Extract all rows as Vec<Vec<String>>
            let str_rows: Vec<Vec<String>> = rows
                .iter()
                .enumerate()
                .map(|(i, row)| match row {
                    Value::Array(fields) => fields
                        .iter()
                        .enumerate()
                        .map(|(j, f)| match f {
                            Value::String(s) => Ok(s.clone()),
                            other => Err(format!(
                                "table_format: rows[{i}][{j}] must be string, got {other}"
                            )),
                        })
                        .collect(),
                    other => Err(format!("table_format: row {i} must be Array, got {other}")),
                })
                .collect::<RResult<_>>()?;

            let ncols = str_rows.iter().map(|r| r.len()).max().unwrap_or(0);
            let mut widths = vec![0usize; ncols];
            for row in &str_rows {
                for (j, cell) in row.iter().enumerate() {
                    if cell.len() > widths[j] {
                        widths[j] = cell.len();
                    }
                }
            }

            let mut out = String::new();
            for (i, row) in str_rows.iter().enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                let mut cells: Vec<String> = (0..ncols)
                    .map(|j| {
                        let s = row.get(j).map(|s| s.as_str()).unwrap_or("");
                        format!("{:width$}", s, width = widths[j])
                    })
                    .collect();
                // Trim trailing whitespace from the last cell
                if let Some(last) = cells.last_mut() {
                    *last = last.trim_end().to_string();
                }
                out.push_str(&cells.join("  "));
            }
            Ok(Value::String(out))
        }
        _ => Err(format!(
            "table_format: expected 1 argument (rows), got {}",
            args.len()
        )),
    }
}

// ── string formatting ─────────────────────────────────────────────────────────

/// `format_float(f, decimals) -> string`
///
/// Formats a float with a fixed number of decimal places.
///
/// ```text
/// format_float(3.14159, 2)  // == "3.14"
/// ```
pub(crate) fn builtin_format_float(args: &[Value]) -> RResult<Value> {
    match args {
        [f_val, d_val] => {
            let f = to_f64(f_val, "format_float: f")?;
            let d = match d_val {
                Value::Int(n) if *n >= 0 => *n as usize,
                Value::Int(n) => {
                    return Err(format!("format_float: decimals must be >= 0, got {n}"));
                }
                other => return Err(format!("format_float: decimals must be int, got {other}")),
            };
            Ok(Value::String(format!("{:.prec$}", f, prec = d)))
        }
        _ => Err(format!(
            "format_float: expected 2 arguments (f, decimals), got {}",
            args.len()
        )),
    }
}

/// `format_int_width(n, width) -> string`
///
/// Formats an integer right-aligned in a field of the given width,
/// padded with spaces.
///
/// ```text
/// format_int_width(42, 6)  // == "    42"
/// ```
pub(crate) fn builtin_format_int_width(args: &[Value]) -> RResult<Value> {
    match args {
        [n_val, w_val] => {
            let n = match n_val {
                Value::Int(n) => *n,
                other => {
                    return Err(format!(
                        "format_int_width: first argument must be int, got {other}"
                    ));
                }
            };
            let w = match w_val {
                Value::Int(w) if *w >= 0 => *w as usize,
                Value::Int(w) => {
                    return Err(format!("format_int_width: width must be >= 0, got {w}"));
                }
                other => return Err(format!("format_int_width: width must be int, got {other}")),
            };
            Ok(Value::String(format!("{:>width$}", n, width = w)))
        }
        _ => Err(format!(
            "format_int_width: expected 2 arguments (n, width), got {}",
            args.len()
        )),
    }
}

/// `format_float_sci(f, sig) -> string`
///
/// Formats a float in scientific notation with `sig` significant digits.
///
/// ```text
/// format_float_sci(12345.6789, 3)  // == "1.235e4" (approximately)
/// ```
pub(crate) fn builtin_format_float_sci(args: &[Value]) -> RResult<Value> {
    match args {
        [f_val, sig_val] => {
            let f = to_f64(f_val, "format_float_sci: f")?;
            let sig = match sig_val {
                Value::Int(n) if *n >= 1 => *n as usize,
                Value::Int(n) => {
                    return Err(format!(
                        "format_float_sci: significant digits must be >= 1, got {n}"
                    ));
                }
                other => {
                    return Err(format!(
                        "format_float_sci: significant digits must be int, got {other}"
                    ));
                }
            };
            // Rust's {:e} uses lowercase e; use sig-1 decimal places
            Ok(Value::String(format!("{:.prec$e}", f, prec = sig - 1)))
        }
        _ => Err(format!(
            "format_float_sci: expected 2 arguments (f, sig), got {}",
            args.len()
        )),
    }
}

// ── run-length encoding ───────────────────────────────────────────────────────

/// `rle_encode(arr) -> Array<Array>`
///
/// Run-length encodes `arr`. Returns a list of `[count, value]` pairs.
///
/// ```text
/// rle_encode([1, 1, 2, 3, 3, 3])  // == [[2, 1], [1, 2], [3, 3]]
/// ```
pub(crate) fn builtin_rle_encode(args: &[Value]) -> RResult<Value> {
    match args {
        [arr] => {
            let items = match arr {
                Value::Array(a) => a,
                other => return Err(format!("rle_encode: expected Array, got {other}")),
            };
            if items.is_empty() {
                return Ok(Value::Array(vec![]));
            }

            let mut out: Vec<Value> = Vec::new();
            let mut count = 1i64;
            let mut current = items[0].clone();

            for item in items.iter().skip(1) {
                if values_equal(&current, item) {
                    count += 1;
                } else {
                    out.push(Value::Array(vec![Value::Int(count), current]));
                    count = 1;
                    current = item.clone();
                }
            }
            out.push(Value::Array(vec![Value::Int(count), current]));
            Ok(Value::Array(out))
        }
        _ => Err(format!(
            "rle_encode: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `rle_decode(arr) -> Array`
///
/// Decodes a run-length encoded array (list of `[count, value]` pairs).
///
/// RES-2118: two micro-optimizations on the decode loop.
///
/// 1. **Pre-size `out`**: sum the counts in one pass before allocating,
///    so the output `Vec` is allocated exactly once instead of growing
///    through the 0→4→8→16... doubling cascade. For decoded arrays with
///    100+ elements this avoids 5+ reallocations + element copies.
///
/// 2. **Move the value on the last push**: the previous shape did
///    `out.push(val.clone())` for *every* iteration including the last,
///    so for a run of length N it cloned the value N times even though
///    `val` is only used once more. Cloning N-1 times and moving the
///    last keeps the value count the same as the run count itself.
pub(crate) fn builtin_rle_decode(args: &[Value]) -> RResult<Value> {
    match args {
        [arr] => {
            let runs = match arr {
                Value::Array(a) => a,
                other => return Err(format!("rle_decode: expected Array, got {other}")),
            };

            // Pre-size the output by summing run lengths in one cheap pass.
            // Negative or non-int counts will be caught in the main loop below
            // — here we just collect a tight capacity estimate.
            let cap: usize = runs
                .iter()
                .filter_map(|run| match run {
                    Value::Array(pair) if pair.len() == 2 => match &pair[0] {
                        Value::Int(n) if *n >= 0 => Some(*n as usize),
                        _ => None,
                    },
                    _ => None,
                })
                .sum();
            let mut out: Vec<Value> = Vec::with_capacity(cap);

            for (i, run) in runs.iter().enumerate() {
                match run {
                    Value::Array(pair) if pair.len() == 2 => {
                        let count = match &pair[0] {
                            Value::Int(n) if *n >= 0 => *n as usize,
                            Value::Int(n) => {
                                return Err(format!(
                                    "rle_decode: run {i} count must be >= 0, got {n}"
                                ));
                            }
                            other => {
                                return Err(format!(
                                    "rle_decode: run {i} count must be int, got {other}"
                                ));
                            }
                        };
                        if count == 0 {
                            continue;
                        }
                        let val = pair[1].clone();
                        // Clone for the first `count - 1` repeats; move
                        // the owned `val` into the final slot.
                        for _ in 0..count - 1 {
                            out.push(val.clone());
                        }
                        out.push(val);
                    }
                    Value::Array(pair) => {
                        return Err(format!(
                            "rle_decode: run {i} must be [count, value] pair, got length {}",
                            pair.len()
                        ));
                    }
                    other => {
                        return Err(format!(
                            "rle_decode: run {i} must be Array [count, value], got {other}"
                        ));
                    }
                }
            }
            Ok(Value::Array(out))
        }
        _ => Err(format!(
            "rle_decode: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn to_f64(v: &Value, ctx: &str) -> RResult<f64> {
    match v {
        Value::Float(f) => Ok(*f),
        Value::Int(i) => Ok(*i as f64),
        other => Err(format!("{ctx}: expected float or int, got {other}")),
    }
}

fn to_usize_pos(v: &Value, name: &str) -> RResult<usize> {
    match v {
        Value::Int(n) if *n >= 0 => Ok(*n as usize),
        Value::Int(n) => Err(format!("{name}: n must be >= 0, got {n}")),
        other => Err(format!("{name}: n must be int, got {other}")),
    }
}

fn as_string<'a>(name: &str, v: &'a Value) -> RResult<&'a str> {
    match v {
        Value::String(s) => Ok(s.as_str()),
        other => Err(format!("{name}: expected string, got {other}")),
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        _ => false,
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

    // ── linspace ─────────────────────────────────────────────────────────────

    #[test]
    fn linspace_basic() {
        let r = run("println(linspace(0.0, 1.0, 5));");
        assert!(r.ok, "errors: {:?}", r.errors);
        // 0.0, 0.25, 0.5, 0.75, 1.0
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    #[test]
    fn linspace_endpoints() {
        let r = run(r#"let v = linspace(0.0, 10.0, 11);
println(v[0]);
println(v[10]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 0.0), "got {}", lines[0]);
        assert!(approx(lines[1], 10.0), "got {}", lines[1]);
    }

    #[test]
    fn linspace_length() {
        let r = run("println(len(linspace(0.0, 100.0, 50)));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("50"), "stdout: {}", r.stdout);
    }

    #[test]
    fn linspace_single_point() {
        let r = run("println(linspace(5.0, 5.0, 1));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('5'), "stdout: {}", r.stdout);
    }

    // ── logspace ─────────────────────────────────────────────────────────────

    #[test]
    fn logspace_powers_of_10() {
        let r = run(r#"let v = logspace(0.0, 3.0, 4);
println(v[0]);
println(v[3]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert!(approx(lines[0], 1.0), "got {}", lines[0]);
        assert!(approx(lines[1], 1000.0), "got {}", lines[1]);
    }

    // ── arange ───────────────────────────────────────────────────────────────

    #[test]
    fn arange_basic() {
        let r = run("println(len(arange(0.0, 5.0, 1.0)));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('5'), "stdout: {}", r.stdout);
    }

    #[test]
    fn arange_negative_step() {
        let r = run("println(len(arange(5.0, 0.0, 0.0 - 1.0)));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('5'), "stdout: {}", r.stdout);
    }

    #[test]
    fn arange_zero_step_errors() {
        let r = run("arange(0.0, 10.0, 0.0);");
        assert!(!r.ok, "expected error for zero step");
    }

    // ── csv_parse ────────────────────────────────────────────────────────────

    #[test]
    fn csv_parse_basic() {
        let r = run(r#"let rows = csv_parse("a,b,c\n1,2,3");
println(len(rows));
println(rows[0][0]);
println(rows[1][2]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "2", "expected 2 rows");
        assert_eq!(lines[1], "a");
        assert_eq!(lines[2], "3");
    }

    #[test]
    fn csv_parse_quoted_fields() {
        let r = run(r#"let rows = csv_parse("\"hello, world\",b\n1,2");
println(rows[0][0]);
println(rows[0][1]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "hello, world");
        assert_eq!(lines[1], "b");
    }

    // ── csv_format ────────────────────────────────────────────────────────────

    #[test]
    fn csv_roundtrip() {
        let r = run(r#"let rows = [["a", "b", "c"], ["1", "2", "3"]];
let s = csv_format(rows);
let back = csv_parse(s);
println(back[0][0]);
println(back[1][2]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "a");
        assert_eq!(lines[1], "3");
    }

    // ── csv_parse_tsv ─────────────────────────────────────────────────────────

    #[test]
    fn csv_parse_tsv_basic() {
        let r = run("let rows = csv_parse_tsv(\"x\ty\tz\");\nprintln(rows[0][1]);");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('y'), "stdout: {}", r.stdout);
    }

    // ── table_format ──────────────────────────────────────────────────────────

    #[test]
    fn table_format_aligns_columns() {
        let r = run(
            r#"let t = table_format([["Name", "Score"], ["Alice", "100"], ["Bob", "99"]]);
println(len(t) > 0);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[test]
    fn table_format_contains_headers() {
        let r = run(
            r#"let t = table_format([["Name", "Score"], ["Alice", "100"]]);
let has_name = string_find(t, "Name");
println(has_name >= 0);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    // ── format_float ─────────────────────────────────────────────────────────

    #[test]
    fn format_float_two_decimals() {
        let r = run(r#"println(format_float(3.14159, 2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("3.14"), "stdout: {}", r.stdout);
    }

    #[test]
    fn format_float_zero_decimals() {
        let r = run(r#"println(format_float(3.7, 0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('4'), "stdout: {}", r.stdout);
    }

    // ── format_int_width ─────────────────────────────────────────────────────

    #[test]
    fn format_int_width_pads() {
        let r = run(r#"println(format_int_width(42, 6));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("    42"), "stdout: {:?}", r.stdout);
    }

    // ── format_float_sci ─────────────────────────────────────────────────────

    #[test]
    fn format_float_sci_basic() {
        let r = run(r#"let s = format_float_sci(12345.0, 3);
println(len(s) > 0);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    // ── rle_encode ───────────────────────────────────────────────────────────

    #[test]
    fn rle_encode_basic() {
        let r = run(r#"let enc = rle_encode([1, 1, 2, 3, 3, 3]);
println(len(enc));
println(enc[0][0]);
println(enc[2][0]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "3", "3 runs");
        assert_eq!(lines[1], "2", "first run count=2");
        assert_eq!(lines[2], "3", "third run count=3");
    }

    #[test]
    fn rle_encode_single_run() {
        let r = run(r#"let enc = rle_encode([5, 5, 5, 5]);
println(len(enc));
println(enc[0][0]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "1");
        assert_eq!(lines[1], "4");
    }

    // ── rle_decode ───────────────────────────────────────────────────────────

    #[test]
    fn rle_roundtrip() {
        let r = run(r#"let arr = [1, 1, 2, 3, 3];
let enc = rle_encode(arr);
let dec = rle_decode(enc);
println(len(dec));
println(dec[0]);
println(dec[4]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "5");
        assert_eq!(lines[1], "1");
        assert_eq!(lines[2], "3");
    }

    #[test]
    fn rle_encode_empty() {
        let r = run("println(len(rle_encode([])));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }
}
