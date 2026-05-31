//! RES-2559: Date/time formatting and parsing builtins (std-only).
//!
//! All functions operate in UTC. Timestamps are seconds since the Unix epoch
//! (1970-01-01 00:00:00 UTC). Nanosecond sub-second precision is stored but
//! only surfaced via the `nanos` field and the `%f` format code.
//!
//! Supported format codes (strftime subset):
//!   %Y — 4-digit year, %m — 2-digit month, %d — 2-digit day,
//!   %H — hour (00-23), %M — minute (00-59), %S — second (00-59),
//!   %f — nanoseconds (9 digits, zero-padded), %% — literal %.

use crate::Value;
use std::time::{SystemTime, UNIX_EPOCH};

type RResult<T> = Result<T, String>;

// ---------------------------------------------------------------------------
// Calendar arithmetic (UTC only, proleptic Gregorian calendar)
// Uses Howard Hinnant's civil_from_days / days_from_civil algorithms.
// ---------------------------------------------------------------------------

fn div_floor(a: i64, b: i64) -> i64 {
    let d = a / b;
    let r = a % b;
    if (r != 0) && ((r < 0) != (b < 0)) {
        d - 1
    } else {
        d
    }
}

/// Convert days-since-Unix-epoch to (year, month [1-12], day [1-31]).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = div_floor(z, 146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Convert (year, month [1-12], day [1-31]) to days-since-Unix-epoch.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let era = div_floor(y, 400);
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * m + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Decompose a Unix timestamp into (year, month, day, hour, minute, second).
fn unix_to_parts(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let day = div_floor(secs, 86_400);
    let time = secs - day * 86_400;
    let (y, mo, d) = civil_from_days(day);
    let h = time / 3_600;
    let mi = (time % 3_600) / 60;
    let s = time % 60;
    (y, mo, d, h, mi, s)
}

/// Compose (year, month, day, hour, minute, second) into a Unix timestamp.
fn parts_to_unix(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
    days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
}

// ---------------------------------------------------------------------------
// Value helpers
// ---------------------------------------------------------------------------

struct DtParts {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    nanos: i64,
}

fn make_datetime(p: &DtParts) -> Value {
    Value::Struct {
        name: "DateTime".to_string(),
        fields: vec![
            ("year".to_string(), Value::Int(p.year)),
            ("month".to_string(), Value::Int(p.month)),
            ("day".to_string(), Value::Int(p.day)),
            ("hour".to_string(), Value::Int(p.hour)),
            ("minute".to_string(), Value::Int(p.minute)),
            ("second".to_string(), Value::Int(p.second)),
            ("nanos".to_string(), Value::Int(p.nanos)),
        ],
    }
}

fn extract_int_field(v: &Value, name: &str) -> RResult<i64> {
    match v {
        Value::Struct { fields, .. } => {
            for (k, val) in fields {
                if k == name {
                    return match val {
                        Value::Int(i) => Ok(*i),
                        other => Err(format!("DateTime.{} must be int, got {:?}", name, other)),
                    };
                }
            }
            Err(format!("DateTime is missing field '{}'", name))
        }
        other => Err(format!("expected DateTime struct, got {:?}", other)),
    }
}

fn unpack_datetime(v: &Value) -> RResult<DtParts> {
    Ok(DtParts {
        year: extract_int_field(v, "year")?,
        month: extract_int_field(v, "month")?,
        day: extract_int_field(v, "day")?,
        hour: extract_int_field(v, "hour")?,
        minute: extract_int_field(v, "minute")?,
        second: extract_int_field(v, "second")?,
        nanos: extract_int_field(v, "nanos")?,
    })
}

fn ok(v: Value) -> Value {
    Value::Result {
        ok: true,
        payload: Box::new(v),
    }
}

fn err(msg: String) -> Value {
    Value::Result {
        ok: false,
        payload: Box::new(Value::String(msg)),
    }
}

// ---------------------------------------------------------------------------
// Format / parse helpers
// ---------------------------------------------------------------------------

fn format_datetime(p: &DtParts, fmt: &str) -> String {
    let mut out = String::with_capacity(fmt.len() + 10);
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('Y') => out.push_str(&format!("{:04}", p.year)),
                Some('m') => out.push_str(&format!("{:02}", p.month)),
                Some('d') => out.push_str(&format!("{:02}", p.day)),
                Some('H') => out.push_str(&format!("{:02}", p.hour)),
                Some('M') => out.push_str(&format!("{:02}", p.minute)),
                Some('S') => out.push_str(&format!("{:02}", p.second)),
                Some('f') => out.push_str(&format!("{:09}", p.nanos)),
                Some('%') => out.push('%'),
                Some(other) => {
                    out.push('%');
                    out.push(other);
                }
                None => out.push('%'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse `s` according to `fmt`. Only numeric format codes are supported.
fn parse_datetime_str(s: &str, fmt: &str) -> RResult<DtParts> {
    let mut year = 1970i64;
    let mut month = 1i64;
    let mut day = 1i64;
    let mut hour = 0i64;
    let mut minute = 0i64;
    let mut second = 0i64;
    let mut nanos = 0i64;

    let sbytes = s.as_bytes();
    let mut si = 0usize;
    let mut fmt_chars = fmt.chars().peekable();

    while let Some(fc) = fmt_chars.next() {
        if fc == '%' {
            match fmt_chars.next() {
                Some('Y') => {
                    let (val, adv) = parse_fixed_int(sbytes, si, 4)?;
                    year = val;
                    si += adv;
                }
                Some('m') => {
                    let (val, adv) = parse_fixed_int(sbytes, si, 2)?;
                    month = val;
                    si += adv;
                }
                Some('d') => {
                    let (val, adv) = parse_fixed_int(sbytes, si, 2)?;
                    day = val;
                    si += adv;
                }
                Some('H') => {
                    let (val, adv) = parse_fixed_int(sbytes, si, 2)?;
                    hour = val;
                    si += adv;
                }
                Some('M') => {
                    let (val, adv) = parse_fixed_int(sbytes, si, 2)?;
                    minute = val;
                    si += adv;
                }
                Some('S') => {
                    let (val, adv) = parse_fixed_int(sbytes, si, 2)?;
                    second = val;
                    si += adv;
                }
                Some('f') => {
                    let (val, adv) = parse_fixed_int(sbytes, si, 9)?;
                    nanos = val;
                    si += adv;
                }
                Some('%') => {
                    if si >= sbytes.len() || sbytes[si] != b'%' {
                        return Err(format!("datetime_parse: expected '%' at position {}", si));
                    }
                    si += 1;
                }
                Some(other) => {
                    return Err(format!("datetime_parse: unknown format code '%{}'", other));
                }
                None => break,
            }
        } else {
            // Literal character — must match exactly.
            if si >= sbytes.len() {
                return Err(format!(
                    "datetime_parse: input too short, expected '{}'",
                    fc
                ));
            }
            if sbytes[si] as char != fc {
                return Err(format!(
                    "datetime_parse: expected '{}' at position {}, got '{}'",
                    fc, si, sbytes[si] as char
                ));
            }
            si += 1;
        }
    }

    Ok(DtParts {
        year,
        month,
        day,
        hour,
        minute,
        second,
        nanos,
    })
}

fn parse_fixed_int(s: &[u8], start: usize, width: usize) -> RResult<(i64, usize)> {
    let end = start + width;
    if end > s.len() {
        return Err(format!(
            "datetime_parse: expected {} digit(s) at position {}",
            width, start
        ));
    }
    let slice = &s[start..end];
    let text = std::str::from_utf8(slice)
        .map_err(|_| format!("datetime_parse: non-UTF-8 at position {}", start))?;
    let val: i64 = text.parse().map_err(|_| {
        format!(
            "datetime_parse: '{}' is not an integer at position {}",
            text, start
        )
    })?;
    Ok((val, width))
}

// ---------------------------------------------------------------------------
// Public builtins
// ---------------------------------------------------------------------------

/// `datetime_now() -> DateTime`
///
/// Returns the current UTC time as a DateTime struct.
pub(crate) fn builtin_datetime_now(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "datetime_now: expected 0 arguments, got {}",
            args.len()
        ));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let nanos = now.subsec_nanos() as i64;
    let (year, month, day, hour, minute, second) = unix_to_parts(secs);
    Ok(make_datetime(&DtParts {
        year,
        month,
        day,
        hour,
        minute,
        second,
        nanos,
    }))
}

/// `datetime_format(dt: DateTime, fmt: string) -> string`
///
/// Formats a DateTime value using strftime-style codes.
pub(crate) fn builtin_datetime_format(args: &[Value]) -> RResult<Value> {
    match args {
        [dt, Value::String(fmt)] => {
            let p = unpack_datetime(dt)?;
            Ok(Value::String(format_datetime(&p, fmt)))
        }
        [_, other] => Err(format!(
            "datetime_format: second argument must be string, got {:?}",
            other
        )),
        _ => Err(format!(
            "datetime_format: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `datetime_parse(s: string, fmt: string) -> Result<DateTime, string>`
///
/// Parses `s` according to `fmt`. Returns `Ok(DateTime)` or `Err(message)`.
pub(crate) fn builtin_datetime_parse(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s), Value::String(fmt)] => match parse_datetime_str(s, fmt) {
            Ok(p) => Ok(ok(make_datetime(&p))),
            Err(e) => Ok(err(e)),
        },
        _ => Err(format!(
            "datetime_parse: expected (string, string), got {} arg(s)",
            args.len()
        )),
    }
}

/// `datetime_to_unix(dt: DateTime) -> int`
///
/// Returns seconds since the Unix epoch (1970-01-01 00:00:00 UTC).
pub(crate) fn builtin_datetime_to_unix(args: &[Value]) -> RResult<Value> {
    match args {
        [dt] => {
            let p = unpack_datetime(dt)?;
            Ok(Value::Int(parts_to_unix(
                p.year, p.month, p.day, p.hour, p.minute, p.second,
            )))
        }
        _ => Err(format!(
            "datetime_to_unix: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `datetime_from_unix(secs: int) -> DateTime`
///
/// Converts a Unix epoch timestamp (seconds) to a UTC DateTime.
pub(crate) fn builtin_datetime_from_unix(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(secs)] => {
            let (year, month, day, hour, minute, second) = unix_to_parts(*secs);
            Ok(make_datetime(&DtParts {
                year,
                month,
                day,
                hour,
                minute,
                second,
                nanos: 0,
            }))
        }
        [other] => Err(format!("datetime_from_unix: expected int, got {:?}", other)),
        _ => Err(format!(
            "datetime_from_unix: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_epoch_is_1970_01_01() {
        let (y, mo, d, h, mi, s) = unix_to_parts(0);
        assert_eq!((y, mo, d, h, mi, s), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn known_date_roundtrip() {
        // 2024-01-15 12:30:45 UTC
        let secs = parts_to_unix(2024, 1, 15, 12, 30, 45);
        let (y, mo, d, h, mi, s) = unix_to_parts(secs);
        assert_eq!((y, mo, d, h, mi, s), (2024, 1, 15, 12, 30, 45));
    }

    #[test]
    fn format_and_parse_roundtrip() {
        let dt = make_datetime(&DtParts {
            year: 2024,
            month: 3,
            day: 7,
            hour: 9,
            minute: 5,
            second: 3,
            nanos: 0,
        });
        if let Value::String(s) =
            builtin_datetime_format(&[dt.clone(), Value::String("%Y-%m-%d %H:%M:%S".into())])
                .unwrap()
        {
            assert_eq!(s, "2024-03-07 09:05:03");
            let parsed = builtin_datetime_parse(&[
                Value::String(s),
                Value::String("%Y-%m-%d %H:%M:%S".into()),
            ])
            .unwrap();
            if let Value::Result {
                ok: true,
                payload: p,
            } = parsed
            {
                let up = unpack_datetime(&p).unwrap();
                assert_eq!(
                    (up.year, up.month, up.day, up.hour, up.minute, up.second, up.nanos),
                    (2024, 3, 7, 9, 5, 3, 0)
                );
            } else {
                panic!("parse returned Err");
            }
        } else {
            panic!("format returned non-string");
        }
    }
}
