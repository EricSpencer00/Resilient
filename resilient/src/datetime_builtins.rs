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

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
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
}
