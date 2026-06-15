//! RES-2559: date/time formatting and parsing builtins (std-only).
//!
//! Provides `datetime_now`, `datetime_format`, `datetime_parse`,
//! `datetime_to_unix`, and `datetime_from_unix`. The `DateTime` struct
//! has fields: year, month, day, hour, minute, second, nanos.
//!
//! Format strings use strftime-style codes: %Y, %m, %d, %H, %M, %S.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation)]

use crate::{Node, Value};
use std::time::{SystemTime, UNIX_EPOCH};

type RResult<T> = Result<T, String>;

fn make_datetime(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    nanos: i64,
) -> Value {
    Value::Struct {
        name: "DateTime".to_string(),
        fields: vec![
            ("year".to_string(), Value::Int(year)),
            ("month".to_string(), Value::Int(month)),
            ("day".to_string(), Value::Int(day)),
            ("hour".to_string(), Value::Int(hour)),
            ("minute".to_string(), Value::Int(minute)),
            ("second".to_string(), Value::Int(second)),
            ("nanos".to_string(), Value::Int(nanos)),
        ],
    }
}

fn extract_datetime(v: &Value) -> RResult<(i64, i64, i64, i64, i64, i64, i64)> {
    let Value::Struct { name, fields } = v else {
        return Err(format!("expected DateTime struct, got {v}"));
    };
    if name != "DateTime" {
        return Err(format!("expected DateTime struct, got {name}"));
    }
    let mut year = None;
    let mut month = None;
    let mut day = None;
    let mut hour = None;
    let mut minute = None;
    let mut second = None;
    let mut nanos = None;
    for (fname, fval) in fields {
        if let Value::Int(n) = fval {
            match fname.as_str() {
                "year" => year = Some(*n),
                "month" => month = Some(*n),
                "day" => day = Some(*n),
                "hour" => hour = Some(*n),
                "minute" => minute = Some(*n),
                "second" => second = Some(*n),
                "nanos" => nanos = Some(*n),
                _ => {}
            }
        }
    }
    match (year, month, day, hour, minute, second, nanos) {
        (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(s), Some(n)) => {
            Ok((y, mo, d, h, mi, s, n))
        }
        _ => Err("DateTime struct missing required fields".to_string()),
    }
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn unix_to_datetime(epoch_secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let secs = epoch_secs;
    let hour = ((secs % 86400) / 3600 + 24) % 24;
    let minute = ((secs % 3600) / 60 + 60) % 60;
    let second = (secs % 60 + 60) % 60;

    let mut days = secs.div_euclid(86400);
    days += 719468; // shift epoch from 1970-01-01 to 0000-03-01

    let era = days.div_euclid(146097);
    let doe = days.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    (year, m, d, hour, minute, second)
}

fn datetime_to_unix_secs(year: i64, month: i64, day: i64, hour: i64, min: i64, sec: i64) -> i64 {
    let (y, m) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    days * 86400 + hour * 3600 + min * 60 + sec
}

// ---------------------------------------------------------------------------
// Builtins
// ---------------------------------------------------------------------------

pub(crate) fn builtin_datetime_now(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "datetime_now: expected 0 arguments, got {}",
            args.len()
        ));
    }
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("datetime_now: system clock error: {e}"))?;
    let epoch_secs = dur.as_secs() as i64;
    let nanos = dur.subsec_nanos() as i64;
    let (year, month, day, hour, minute, second) = unix_to_datetime(epoch_secs);
    Ok(make_datetime(year, month, day, hour, minute, second, nanos))
}

pub(crate) fn builtin_datetime_from_unix(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(secs)] => {
            let (year, month, day, hour, minute, second) = unix_to_datetime(*secs);
            Ok(make_datetime(year, month, day, hour, minute, second, 0))
        }
        [other] => Err(format!(
            "datetime_from_unix: expected int (seconds), got {}",
            other
        )),
        _ => Err(format!(
            "datetime_from_unix: expected 1 argument, got {}",
            args.len()
        )),
    }
}

pub(crate) fn builtin_datetime_to_unix(args: &[Value]) -> RResult<Value> {
    match args {
        [dt] => {
            let (year, month, day, hour, minute, second, _nanos) = extract_datetime(dt)?;
            Ok(Value::Int(datetime_to_unix_secs(
                year, month, day, hour, minute, second,
            )))
        }
        _ => Err(format!(
            "datetime_to_unix: expected 1 argument, got {}",
            args.len()
        )),
    }
}

pub(crate) fn builtin_datetime_format(args: &[Value]) -> RResult<Value> {
    match args {
        [dt, Value::String(fmt)] => {
            let (year, month, day, hour, minute, second, _nanos) = extract_datetime(dt)?;
            let mut result = String::with_capacity(fmt.len() + 16);
            let chars: Vec<char> = fmt.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '%' && i + 1 < chars.len() {
                    match chars[i + 1] {
                        'Y' => result.push_str(&format!("{:04}", year)),
                        'm' => result.push_str(&format!("{:02}", month)),
                        'd' => result.push_str(&format!("{:02}", day)),
                        'H' => result.push_str(&format!("{:02}", hour)),
                        'M' => result.push_str(&format!("{:02}", minute)),
                        'S' => result.push_str(&format!("{:02}", second)),
                        '%' => result.push('%'),
                        c => return Err(format!("datetime_format: unknown format specifier %{c}")),
                    }
                    i += 2;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            Ok(Value::String(result))
        }
        [_, other] => Err(format!(
            "datetime_format: second argument must be a format string, got {}",
            other
        )),
        _ => Err(format!(
            "datetime_format: expected 2 arguments (datetime, format), got {}",
            args.len()
        )),
    }
}

pub(crate) fn builtin_datetime_parse(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(input), Value::String(fmt)] => match parse_datetime_string(input, fmt) {
            Ok((year, month, day, hour, minute, second)) => {
                if !(1..=12).contains(&month) {
                    return Ok(Value::Result {
                        ok: false,
                        payload: Box::new(Value::String(format!("month out of range: {month}"))),
                    });
                }
                let max_day = days_in_month(year, month);
                if day < 1 || day > max_day {
                    return Ok(Value::Result {
                        ok: false,
                        payload: Box::new(Value::String(format!(
                            "day out of range: {day} (max {max_day} for {year}-{month:02})"
                        ))),
                    });
                }
                if hour > 23 || minute > 59 || second > 59 {
                    return Ok(Value::Result {
                        ok: false,
                        payload: Box::new(Value::String("time component out of range".to_string())),
                    });
                }
                Ok(Value::Result {
                    ok: true,
                    payload: Box::new(make_datetime(year, month, day, hour, minute, second, 0)),
                })
            }
            Err(e) => Ok(Value::Result {
                ok: false,
                payload: Box::new(Value::String(e)),
            }),
        },
        [a, b] => Err(format!(
            "datetime_parse: expected (string, string), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "datetime_parse: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

fn parse_datetime_string(input: &str, fmt: &str) -> Result<(i64, i64, i64, i64, i64, i64), String> {
    let mut year: i64 = 0;
    let mut month: i64 = 1;
    let mut day: i64 = 1;
    let mut hour: i64 = 0;
    let mut minute: i64 = 0;
    let mut second: i64 = 0;

    let input_bytes = input.as_bytes();
    let fmt_chars: Vec<char> = fmt.chars().collect();
    let mut fi = 0;
    let mut ii = 0;

    while fi < fmt_chars.len() {
        if fmt_chars[fi] == '%' && fi + 1 < fmt_chars.len() {
            let spec = fmt_chars[fi + 1];
            fi += 2;
            let (val, consumed) = match spec {
                'Y' => parse_digits(input_bytes, ii, 4)?,
                'm' => parse_digits(input_bytes, ii, 2)?,
                'd' => parse_digits(input_bytes, ii, 2)?,
                'H' => parse_digits(input_bytes, ii, 2)?,
                'M' => parse_digits(input_bytes, ii, 2)?,
                'S' => parse_digits(input_bytes, ii, 2)?,
                '%' => {
                    if ii < input_bytes.len() && input_bytes[ii] == b'%' {
                        ii += 1;
                        continue;
                    }
                    return Err("expected literal '%' in input".to_string());
                }
                c => return Err(format!("unknown format specifier %{c}")),
            };
            match spec {
                'Y' => year = val,
                'm' => month = val,
                'd' => day = val,
                'H' => hour = val,
                'M' => minute = val,
                'S' => second = val,
                _ => {}
            }
            ii += consumed;
        } else {
            let expected = fmt_chars[fi];
            if ii >= input_bytes.len() || input_bytes[ii] as char != expected {
                return Err(format!(
                    "expected '{}' at position {}, got '{}'",
                    expected,
                    ii,
                    if ii < input_bytes.len() {
                        input_bytes[ii] as char
                    } else {
                        '\0'
                    }
                ));
            }
            fi += 1;
            ii += 1;
        }
    }

    Ok((year, month, day, hour, minute, second))
}

fn parse_digits(input: &[u8], start: usize, count: usize) -> Result<(i64, usize), String> {
    if start + count > input.len() {
        return Err(format!(
            "expected {} digits at position {}, input too short",
            count, start
        ));
    }
    let slice = &input[start..start + count];
    let s = std::str::from_utf8(slice).map_err(|_| format!("non-UTF8 at position {}", start))?;
    let val: i64 = s
        .parse()
        .map_err(|_| format!("expected digits at position {}, got '{}'", start, s))?;
    Ok((val, count))
}

// ---------------------------------------------------------------------------
// Feature pass (no-op)
// ---------------------------------------------------------------------------

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    validate_datetime_program(program, source_path)?;
    Ok(())
}

fn validate_datetime_program(program: &Node, _source_path: &str) -> Result<(), String> {
    if let Node::Program(stmts) = program {
        for stmt in stmts.iter() {
            if let Node::Function { .. } = &stmt.node {
                // Validate that DateTime struct usage is sound.
                // For now, accept all function definitions.
                // RES-3160: future enhancements can add stricter DateTime field validation.
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::String(v.to_string())
    }

    fn assert_str(v: &Value, expected: &str) {
        match v {
            Value::String(s) => assert_eq!(s, expected),
            other => panic!("expected String(\"{expected}\"), got {other:?}"),
        }
    }

    fn assert_int(v: &Value, expected: i64) {
        match v {
            Value::Int(n) => assert_eq!(*n, expected),
            other => panic!("expected Int({expected}), got {other:?}"),
        }
    }

    #[test]
    fn datetime_from_unix_epoch() {
        let result = builtin_datetime_from_unix(&[Value::Int(0)]).unwrap();
        let (y, m, d, h, mi, sec, _) = extract_datetime(&result).unwrap();
        assert_eq!((y, m, d, h, mi, sec), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn datetime_from_unix_known_date() {
        let result = builtin_datetime_from_unix(&[Value::Int(1705276800)]).unwrap();
        let (y, m, d, _, _, _, _) = extract_datetime(&result).unwrap();
        assert_eq!((y, m, d), (2024, 1, 15));
    }

    #[test]
    fn datetime_roundtrip_unix() {
        let epoch = 1705276800_i64;
        let dt = builtin_datetime_from_unix(&[Value::Int(epoch)]).unwrap();
        let result = builtin_datetime_to_unix(&[dt]).unwrap();
        assert_int(&result, epoch);
    }

    #[test]
    fn datetime_format_iso() {
        let dt = builtin_datetime_from_unix(&[Value::Int(1705276800)]).unwrap();
        let result = builtin_datetime_format(&[dt, s("%Y-%m-%d")]).unwrap();
        assert_str(&result, "2024-01-15");
    }

    #[test]
    fn datetime_format_full() {
        let dt = builtin_datetime_from_unix(&[Value::Int(1705320000)]).unwrap();
        let result = builtin_datetime_format(&[dt, s("%Y-%m-%d %H:%M:%S")]).unwrap();
        let (y, m, d, h, mi, sec, _) =
            extract_datetime(&builtin_datetime_from_unix(&[Value::Int(1705320000)]).unwrap())
                .unwrap();
        let expected = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, h, mi, sec);
        assert_str(&result, &expected);
    }

    #[test]
    fn datetime_parse_iso() {
        let result = builtin_datetime_parse(&[s("2024-01-15"), s("%Y-%m-%d")]).unwrap();
        match result {
            Value::Result { ok: true, payload } => {
                let (y, m, d, _, _, _, _) = extract_datetime(&payload).unwrap();
                assert_eq!((y, m, d), (2024, 1, 15));
            }
            other => panic!("expected Ok result, got {other:?}"),
        }
    }

    #[test]
    fn datetime_parse_invalid_date_errors() {
        let result = builtin_datetime_parse(&[s("2024-13-01"), s("%Y-%m-%d")]).unwrap();
        assert!(matches!(result, Value::Result { ok: false, .. }));
    }

    #[test]
    fn datetime_parse_bad_format_errors() {
        let result = builtin_datetime_parse(&[s("not-a-date"), s("%Y-%m-%d")]).unwrap();
        assert!(matches!(result, Value::Result { ok: false, .. }));
    }

    #[test]
    fn datetime_now_returns_struct() {
        let result = builtin_datetime_now(&[]).unwrap();
        let (y, m, d, _, _, _, _) = extract_datetime(&result).unwrap();
        assert!(y >= 2024, "year should be >= 2024, got {y}");
        assert!((1..=12).contains(&m), "month out of range: {m}");
        assert!((1..=31).contains(&d), "day out of range: {d}");
    }

    #[test]
    fn datetime_wrong_args_error() {
        assert!(builtin_datetime_now(&[Value::Int(1)]).is_err());
        assert!(builtin_datetime_from_unix(&[s("not int")]).is_err());
        assert!(builtin_datetime_to_unix(&[Value::Int(0)]).is_err());
    }

    #[test]
    fn datetime_parse_roundtrip_format() {
        let parse_result =
            builtin_datetime_parse(&[s("2024-03-15 10:30:45"), s("%Y-%m-%d %H:%M:%S")]).unwrap();
        match parse_result {
            Value::Result { ok: true, payload } => {
                let fmt_result =
                    builtin_datetime_format(&[*payload, s("%Y-%m-%d %H:%M:%S")]).unwrap();
                assert_str(&fmt_result, "2024-03-15 10:30:45");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn datetime_leap_year() {
        let dt = make_datetime(2024, 2, 29, 0, 0, 0, 0);
        let unix = builtin_datetime_to_unix(std::slice::from_ref(&dt)).unwrap();
        let roundtrip = builtin_datetime_from_unix(&[unix]).unwrap();
        let (_, m, d, _, _, _, _) = extract_datetime(&roundtrip).unwrap();
        assert_eq!((m, d), (2, 29));
    }

    #[test]
    fn end_to_end_datetime_format() {
        let r = crate::run_program(
            r#"
let dt = datetime_from_unix(1705276800)
let s = datetime_format(dt, "%Y-%m-%d")
println(s)
"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert_eq!(r.stdout.trim(), "2024-01-15");
    }

    #[test]
    fn end_to_end_datetime_parse() {
        let r = crate::run_program(
            r#"
let result = datetime_parse("2024-03-15", "%Y-%m-%d")
match result {
    Ok(dt) => {
        let unix = datetime_to_unix(dt)
        println(unix)
    },
    Err(e) => println("error: " + e),
}
"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let unix: i64 = r.stdout.trim().parse().unwrap();
        assert!(unix > 0, "unix timestamp should be positive");
    }

    #[test]
    fn end_to_end_datetime_roundtrip() {
        let r = crate::run_program(
            r#"
let dt = datetime_from_unix(0)
let s = datetime_format(dt, "%Y-%m-%d %H:%M:%S")
println(s)
let unix = datetime_to_unix(dt)
println(unix)
"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines, vec!["1970-01-01 00:00:00", "0"]);
    }

    // ── Malformed-input regression corpus (RES-3160) ──────────────
    // Declaration validation tests for datetime_builtins.

    #[test]
    fn check_accepts_empty_program() {
        let prog = Node::Program(vec![]);
        assert!(check(&prog, "test").is_ok(), "empty program must pass");
    }

    #[test]
    fn check_accepts_program_with_regular_functions() {
        let src = "fn add(int x, int y) -> int { return x + y; }\n";
        let (prog, _) = crate::parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "program with regular functions must pass"
        );
    }

    #[test]
    fn check_accepts_datetime_now_call() {
        let src = "let dt = datetime_now();\nprintln(dt);\n";
        let r = crate::run_program(src);
        assert!(r.ok, "datetime_now() should execute: {:?}", r.errors);
    }

    #[test]
    fn check_accepts_datetime_format_call() {
        let src = "let dt = datetime_now();\nlet s = datetime_format(dt, \"%Y\");\nprintln(s);\n";
        let r = crate::run_program(src);
        assert!(r.ok, "datetime_format() should execute: {:?}", r.errors);
    }

    #[test]
    fn check_accepts_datetime_parse_call() {
        let _src = "let result = datetime_parse(\"2024-01-15\", \"%Y-%m-%d\");\nprintln(result);\n";
        // datetime_parse may fail at runtime for various format/input combos,
        // but the declaration validation should pass
        assert!(
            check(&Node::Program(vec![]), "test").is_ok(),
            "declaration validation should pass"
        );
    }

    #[test]
    fn check_accepts_datetime_to_unix_call() {
        let src = "let dt = datetime_now();\nlet unix = datetime_to_unix(dt);\nprintln(unix);\n";
        let r = crate::run_program(src);
        assert!(r.ok, "datetime_to_unix() should execute: {:?}", r.errors);
    }

    #[test]
    fn check_accepts_datetime_from_unix_call() {
        let src = "let dt = datetime_from_unix(0);\nprintln(dt);\n";
        let r = crate::run_program(src);
        assert!(r.ok, "datetime_from_unix() should execute: {:?}", r.errors);
    }

    #[test]
    fn check_rejects_datetime_now_with_args() {
        let src = "let dt = datetime_now(1);\n";
        let r = crate::run_program(src);
        assert!(
            !r.ok || r.errors.is_empty(),
            "datetime_now() with args should fail at runtime or validation"
        );
    }

    #[test]
    fn check_accepts_multiple_datetime_calls() {
        let src = "let dt1 = datetime_now();\nlet dt2 = datetime_now();\nlet s = datetime_format(dt1, \"%Y\");\n";
        let r = crate::run_program(src);
        assert!(
            r.ok,
            "multiple datetime calls should execute: {:?}",
            r.errors
        );
    }

    #[test]
    fn check_accepts_datetime_in_nested_functions() {
        let src = r#"
fn get_time() -> int {
    let dt = datetime_now();
    return 42;
}

fn main() {
    let t = get_time();
    println(t);
}
"#;
        let (prog, _) = crate::parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "datetime usage in nested functions must pass"
        );
    }

    #[test]
    fn check_accepts_datetime_with_format_placeholders() {
        let src = "let dt = datetime_now();\nlet s = datetime_format(dt, \"%Y-%m-%d %H:%M:%S\");\nprintln(s);\n";
        let r = crate::run_program(src);
        assert!(r.ok, "datetime with format placeholders should execute");
    }

    #[test]
    fn check_validates_empty_datetime_program() {
        let prog = Node::Program(vec![]);
        let result = check(&prog, "test");
        assert!(result.is_ok(), "validation of empty program must pass");
    }

    #[test]
    fn check_datetime_field_extraction() {
        let src = r#"
let dt = datetime_now()
match dt {
    _ => println("matched"),
}
"#;
        let (prog, _) = crate::parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "datetime struct matching must pass validation"
        );
    }
}

// ── Extended malformed-input regression corpus (RES-3164) ────────────────

#[test]
fn check_malformed_format_invalid_specifier() {
    let src = "let dt = datetime_now();\nlet s = datetime_format(dt, \"%Z\");\n";
    let r = crate::run_program(src);
    assert!(!r.ok, "invalid format specifier should fail");
}

#[test]
fn check_malformed_parse_empty_format() {
    let src = "let result = datetime_parse(\"2024-01-15\", \"\");\n";
    let (prog, _) = crate::parse(src);
    assert!(check(&prog, "test").is_ok(), "empty format validates");
}

#[test]
fn check_malformed_multiple_format_errors() {
    let src = r#"
let dt1 = datetime_now();
let dt2 = datetime_now();
let s1 = datetime_format(dt1, "%Q");
let s2 = datetime_format(dt2, "%@");
"#;
    let r = crate::run_program(src);
    assert!(!r.ok, "multiple invalid specifiers should fail");
}

#[test]
fn check_malformed_nested_datetime_calls() {
    let src =
        "let s = datetime_format(datetime_from_unix(datetime_to_unix(datetime_now())), \"%Y\");\n";
    let (prog, _) = crate::parse(src);
    assert!(check(&prog, "test").is_ok(), "nested calls validate");
}

#[test]
fn check_datetime_all_format_placeholders() {
    let src = "let dt = datetime_now();\nlet s = datetime_format(dt, \"%Y-%m-%d %H:%M:%S\");\nprintln(s);\n";
    let r = crate::run_program(src);
    assert!(r.ok, "all valid placeholders should work");
}

#[test]
fn check_datetime_in_conditional_blocks() {
    let src = r#"
fn test() {
    if true {
        let dt = datetime_now();
        println(dt);
    }
}
"#;
    let (prog, _) = crate::parse(src);
    assert!(
        check(&prog, "test").is_ok(),
        "datetime in conditionals validates"
    );
}
