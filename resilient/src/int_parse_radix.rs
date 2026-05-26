//! Integer parsing from hex/binary strings and octal formatting.
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `int_parse_hex(s)` | `(String) -> Int` | parse `"ff"` / `"0xff"` → 255 |
//! | `int_parse_bin(s)` | `(String) -> Int` | parse `"1010"` / `"0b1010"` → 10 |
//! | `int_to_oct(n)` | `(Int) -> String` | format `255` → `"377"` (octal) |
//!
//! Pairs with the existing `int_to_hex` / `int_to_bin` formatters.

use crate::{RResult, Value};

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> &'a str {
    // Strip `0x` / `0X` / `0b` / `0B` prefix — case-insensitive.
    s.strip_prefix(prefix)
        .or_else(|| s.strip_prefix(&prefix.to_uppercase()))
        .unwrap_or(s)
}

/// `int_parse_hex(s)` — parse a hexadecimal string (case-insensitive, with or
/// without `0x` prefix) and return the integer value.
pub(crate) fn builtin_int_parse_hex(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => {
            let stripped = strip_prefix_ci(s.trim(), "0x");
            i64::from_str_radix(stripped, 16)
                .map(Value::Int)
                .map_err(|_| format!("int_parse_hex: cannot parse {:?} as hex integer", s))
        }
        [other] => Err(format!("int_parse_hex: expected a String, got {}", other)),
        _ => Err(format!(
            "int_parse_hex: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `int_parse_bin(s)` — parse a binary string (with or without `0b` prefix)
/// and return the integer value.
pub(crate) fn builtin_int_parse_bin(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => {
            let stripped = strip_prefix_ci(s.trim(), "0b");
            i64::from_str_radix(stripped, 2)
                .map(Value::Int)
                .map_err(|_| format!("int_parse_bin: cannot parse {:?} as binary integer", s))
        }
        [other] => Err(format!("int_parse_bin: expected a String, got {}", other)),
        _ => Err(format!(
            "int_parse_bin: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `int_to_oct(n)` — format `n` as an octal string (no prefix). Negative
/// values use Rust's `{:o}` representation.
pub(crate) fn builtin_int_to_oct(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => Ok(Value::String(format!("{:o}", n))),
        [other] => Err(format!("int_to_oct: expected an Int, got {}", other)),
        _ => Err(format!(
            "int_to_oct: expected 1 argument, got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unwrap_int(v: Value) -> i64 {
        match v {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    fn unwrap_string(v: Value) -> String {
        match v {
            Value::String(s) => s,
            other => panic!("expected String, got {:?}", other),
        }
    }

    // --- int_parse_hex ---

    #[test]
    fn parse_hex_lowercase() {
        assert_eq!(
            unwrap_int(builtin_int_parse_hex(&[Value::String("ff".to_string())]).unwrap()),
            255
        );
    }

    #[test]
    fn parse_hex_uppercase() {
        assert_eq!(
            unwrap_int(builtin_int_parse_hex(&[Value::String("FF".to_string())]).unwrap()),
            255
        );
    }

    #[test]
    fn parse_hex_with_0x_prefix() {
        assert_eq!(
            unwrap_int(builtin_int_parse_hex(&[Value::String("0xff".to_string())]).unwrap()),
            255
        );
    }

    #[test]
    fn parse_hex_with_0x_upper_prefix() {
        assert_eq!(
            unwrap_int(builtin_int_parse_hex(&[Value::String("0XFF".to_string())]).unwrap()),
            255
        );
    }

    #[test]
    fn parse_hex_zero() {
        assert_eq!(
            unwrap_int(builtin_int_parse_hex(&[Value::String("0".to_string())]).unwrap()),
            0
        );
    }

    #[test]
    fn parse_hex_invalid() {
        let err = builtin_int_parse_hex(&[Value::String("xyz".to_string())]).unwrap_err();
        assert!(err.contains("cannot parse"), "{}", err);
    }

    #[test]
    fn parse_hex_rejects_non_string() {
        let err = builtin_int_parse_hex(&[Value::Int(255)]).unwrap_err();
        assert!(err.contains("expected a String"), "{}", err);
    }

    // --- int_parse_bin ---

    #[test]
    fn parse_bin_basic() {
        assert_eq!(
            unwrap_int(builtin_int_parse_bin(&[Value::String("1010".to_string())]).unwrap()),
            10
        );
    }

    #[test]
    fn parse_bin_with_0b_prefix() {
        assert_eq!(
            unwrap_int(builtin_int_parse_bin(&[Value::String("0b1010".to_string())]).unwrap()),
            10
        );
    }

    #[test]
    fn parse_bin_with_0b_upper_prefix() {
        assert_eq!(
            unwrap_int(builtin_int_parse_bin(&[Value::String("0B1010".to_string())]).unwrap()),
            10
        );
    }

    #[test]
    fn parse_bin_zero() {
        assert_eq!(
            unwrap_int(builtin_int_parse_bin(&[Value::String("0".to_string())]).unwrap()),
            0
        );
    }

    #[test]
    fn parse_bin_invalid() {
        let err = builtin_int_parse_bin(&[Value::String("102".to_string())]).unwrap_err();
        assert!(err.contains("cannot parse"), "{}", err);
    }

    // --- int_to_oct ---

    #[test]
    fn oct_255() {
        assert_eq!(
            unwrap_string(builtin_int_to_oct(&[Value::Int(255)]).unwrap()),
            "377"
        );
    }

    #[test]
    fn oct_zero() {
        assert_eq!(
            unwrap_string(builtin_int_to_oct(&[Value::Int(0)]).unwrap()),
            "0"
        );
    }

    #[test]
    fn oct_eight() {
        assert_eq!(
            unwrap_string(builtin_int_to_oct(&[Value::Int(8)]).unwrap()),
            "10"
        );
    }

    #[test]
    fn oct_rejects_non_int() {
        let err = builtin_int_to_oct(&[Value::String("ff".to_string())]).unwrap_err();
        assert!(err.contains("expected an Int"), "{}", err);
    }
}
