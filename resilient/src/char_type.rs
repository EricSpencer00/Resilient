//! RES-2619: `Char` — 32-bit Unicode scalar value type.
//!
//! Adds `'A'`, `'\n'`, `'\u{1F600}'` single-quoted char literals, a
//! `Value::Char(char)` runtime value, and a suite of character-level
//! builtins.
//!
//! ## Usage
//!
//! ```text
//! let c: Char = 'A';
//! println(char_is_alpha(c));    // true
//! println(char_to_lower(c));    // 'a'
//! println(char_to_int(c));      // 65
//! let d = int_to_char(66);      // 'B'
//! ```
//!
//! ## Feature isolation
//!
//! All builtin implementations live here. Core files add only:
//!
//! - `lexer_logos.rs`: `CharLit(char)` Tok variant + `char_lit` callback +
//!   `Tok::CharLit(c) => Token::CharLiteral(c)` mapping.
//! - `lib.rs Token`: `CharLiteral(char)` literal variant (Literals section).
//! - `lib.rs Node`: `CharLiteral { value: char, span }` AST variant.
//! - `lib.rs Value`: `Char(char)` variant + Display + Debug arms.
//! - `lib.rs parse_expression`: arm for `Token::CharLiteral(c)`.
//! - `lib.rs eval`: arm for `Node::CharLiteral`.
//! - `lib.rs BUILTINS`: entries for all `char_*` / `int_to_char` functions.
//! - `type_builtins.rs`: `Value::Char(_) => "char"` arm in `type_of`.

use crate::Value;

type RResult<T> = Result<T, String>;

// ── helpers ──────────────────────────────────────────────────────────────────

fn expect_char(v: &Value, fn_name: &str) -> RResult<char> {
    match v {
        Value::Char(c) => Ok(*c),
        other => Err(format!("{fn_name}: expected Char, got {other}")),
    }
}

// ── classification ────────────────────────────────────────────────────────────

/// `char_is_alpha(c)` — true if `c` is an alphabetic Unicode character.
pub(crate) fn builtin_char_is_alpha(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Bool(
            expect_char(v, "char_is_alpha")?.is_alphabetic(),
        )),
        _ => Err(format!(
            "char_is_alpha: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `char_is_digit(c)` — true if `c` is an ASCII decimal digit (0–9).
pub(crate) fn builtin_char_is_digit(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Bool(
            expect_char(v, "char_is_digit")?.is_ascii_digit(),
        )),
        _ => Err(format!(
            "char_is_digit: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `char_is_whitespace(c)` — true if `c` is a Unicode whitespace character.
pub(crate) fn builtin_char_is_whitespace(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Bool(
            expect_char(v, "char_is_whitespace")?.is_whitespace(),
        )),
        _ => Err(format!(
            "char_is_whitespace: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `char_is_upper(c)` — true if `c` is an uppercase Unicode letter.
pub(crate) fn builtin_char_is_upper(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Bool(expect_char(v, "char_is_upper")?.is_uppercase())),
        _ => Err(format!(
            "char_is_upper: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `char_is_lower(c)` — true if `c` is a lowercase Unicode letter.
pub(crate) fn builtin_char_is_lower(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Bool(expect_char(v, "char_is_lower")?.is_lowercase())),
        _ => Err(format!(
            "char_is_lower: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `char_is_alphanumeric(c)` — true if `c` is alphanumeric.
pub(crate) fn builtin_char_is_alphanumeric(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Bool(
            expect_char(v, "char_is_alphanumeric")?.is_alphanumeric(),
        )),
        _ => Err(format!(
            "char_is_alphanumeric: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `char_is_ascii(c)` — true if `c` is an ASCII character (code point ≤ 127).
pub(crate) fn builtin_char_is_ascii(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Bool(expect_char(v, "char_is_ascii")?.is_ascii())),
        _ => Err(format!(
            "char_is_ascii: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ── case conversion ───────────────────────────────────────────────────────────

/// `char_to_upper(c)` — uppercase version of `c`, or `c` if no mapping.
///
/// Uses Rust's `to_uppercase()` iterator; if the mapping produces multiple
/// characters (e.g., the German ß → SS), returns the first.
pub(crate) fn builtin_char_to_upper(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let c = expect_char(v, "char_to_upper")?;
            let upper = c.to_uppercase().next().unwrap_or(c);
            Ok(Value::Char(upper))
        }
        _ => Err(format!(
            "char_to_upper: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `char_to_lower(c)` — lowercase version of `c`, or `c` if no mapping.
pub(crate) fn builtin_char_to_lower(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => {
            let c = expect_char(v, "char_to_lower")?;
            let lower = c.to_lowercase().next().unwrap_or(c);
            Ok(Value::Char(lower))
        }
        _ => Err(format!(
            "char_to_lower: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ── conversion ────────────────────────────────────────────────────────────────

/// `char_to_int(c)` — Unicode code point as an integer.
///
/// `char_to_int('A')` → 65, `char_to_int('🎉')` → 127881.
pub(crate) fn builtin_char_to_int(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Int(expect_char(v, "char_to_int")? as i64)),
        _ => Err(format!(
            "char_to_int: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `int_to_char(n)` — char from a Unicode code point, or error if invalid.
///
/// `int_to_char(65)` → `'A'`, `int_to_char(0xD800)` → error (surrogate).
pub(crate) fn builtin_int_to_char(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            if *n < 0 || *n > i64::from(u32::MAX) {
                return Err(format!("int_to_char: code point {} out of range", n));
            }
            match char::from_u32(*n as u32) {
                Some(c) => Ok(Value::Char(c)),
                None => Err(format!(
                    "int_to_char: 0x{:X} is not a valid Unicode scalar value",
                    n
                )),
            }
        }
        [other] => Err(format!("int_to_char: expected Int, got {other}")),
        _ => Err(format!(
            "int_to_char: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `char_to_string(c)` — convert a Char to a one-character String.
pub(crate) fn builtin_char_to_string(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::String(expect_char(v, "char_to_string")?.to_string())),
        _ => Err(format!(
            "char_to_string: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// Parse a single char from the inside of a `'...'` literal (quotes already
/// stripped). Returns `None` if the content does not decode to exactly one
/// Unicode scalar value.
///
/// Recognised escape sequences:
/// - `\n`, `\t`, `\r`, `\0`, `\\`, `\'`
/// - `\xHH` (two hex digits, 0x00–0xFF)
/// - `\u{HHHH}` (1–6 hex digits, any valid Unicode scalar)
///
/// Any other character sequence (including multi-char or empty) returns `None`.
pub(crate) fn parse_char_inner(inner: &str) -> Option<char> {
    if inner.is_empty() {
        return None;
    }
    let mut chars = inner.chars().peekable();
    let result = if chars.peek() == Some(&'\\') {
        chars.next(); // consume '\'
        match chars.next()? {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            '0' => '\0',
            '\\' => '\\',
            '\'' => '\'',
            '"' => '"',
            'x' => {
                let h1 = chars.next()?;
                let h2 = chars.next()?;
                if h1.is_ascii_hexdigit() && h2.is_ascii_hexdigit() {
                    let byte = u8::from_str_radix(&format!("{h1}{h2}"), 16).ok()?;
                    byte as char
                } else {
                    return None;
                }
            }
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut hex = String::with_capacity(6);
                loop {
                    match chars.peek()? {
                        '}' => {
                            chars.next();
                            break;
                        }
                        d if d.is_ascii_hexdigit() => {
                            hex.push(*d);
                            chars.next();
                        }
                        _ => return None,
                    }
                }
                let n = u32::from_str_radix(&hex, 16).ok()?;
                char::from_u32(n)?
            }
            other => other,
        }
    } else {
        chars.next()?
    };
    // Ensure nothing is left — exactly one char.
    if chars.next().is_some() {
        return None;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── parse_char_inner ─────────────────────────────────────────────────────

    #[test]
    fn parse_single_ascii() {
        assert_eq!(parse_char_inner("A"), Some('A'));
        assert_eq!(parse_char_inner("z"), Some('z'));
        assert_eq!(parse_char_inner("0"), Some('0'));
        assert_eq!(parse_char_inner(" "), Some(' '));
    }

    #[test]
    fn parse_escape_sequences() {
        assert_eq!(parse_char_inner("\\n"), Some('\n'));
        assert_eq!(parse_char_inner("\\t"), Some('\t'));
        assert_eq!(parse_char_inner("\\r"), Some('\r'));
        assert_eq!(parse_char_inner("\\0"), Some('\0'));
        assert_eq!(parse_char_inner("\\\\"), Some('\\'));
        assert_eq!(parse_char_inner("\\'"), Some('\''));
    }

    #[test]
    fn parse_hex_escape() {
        assert_eq!(parse_char_inner("\\x41"), Some('A'));
        assert_eq!(parse_char_inner("\\x00"), Some('\0'));
        assert_eq!(parse_char_inner("\\xFF"), Some('ÿ'));
    }

    #[test]
    fn parse_unicode_escape() {
        assert_eq!(parse_char_inner("\\u{41}"), Some('A'));
        assert_eq!(parse_char_inner("\\u{1F600}"), Some('😀'));
        assert_eq!(parse_char_inner("\\u{0}"), Some('\0'));
    }

    #[test]
    fn parse_empty_or_multi_returns_none() {
        assert_eq!(parse_char_inner(""), None);
        assert_eq!(parse_char_inner("AB"), None);
        assert_eq!(parse_char_inner("\\nA"), None);
    }

    // ── end-to-end interpreter tests ─────────────────────────────────────────

    #[test]
    fn char_literal_basic() {
        let r = run("println(type_of('A'));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("char"), "stdout: {}", r.stdout);
    }

    #[test]
    fn char_to_int_and_back() {
        let r = run("println(char_to_int('A')); println(int_to_char(65));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "65");
        assert_eq!(lines[1], "A");
    }

    #[test]
    fn char_classification() {
        let r = run(r#"
println(char_is_alpha('A'));
println(char_is_digit('5'));
println(char_is_whitespace(' '));
println(char_is_upper('Z'));
println(char_is_lower('a'));
println(char_is_alpha('1'));
println(char_is_digit('x'));
"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "true");
        assert_eq!(lines[2], "true");
        assert_eq!(lines[3], "true");
        assert_eq!(lines[4], "true");
        assert_eq!(lines[5], "false");
        assert_eq!(lines[6], "false");
    }

    #[test]
    fn char_case_conversion() {
        let r = run("println(char_to_upper('a')); println(char_to_lower('Z'));");
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "A");
        assert_eq!(lines[1], "z");
    }

    #[test]
    fn char_escape_newline() {
        let r = run("let c = '\\n'; println(char_to_int(c));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("10"), "stdout: {}", r.stdout);
    }

    #[test]
    fn char_unicode_escape() {
        let r = run("let c = '\\u{41}'; println(c);");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('A'), "stdout: {}", r.stdout);
    }

    #[test]
    fn char_to_string_builtin() {
        let r = run("println(char_to_string('H'));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('H'), "stdout: {}", r.stdout);
    }

    #[test]
    fn int_to_char_invalid() {
        // 0xD800 is a surrogate — not a valid Unicode scalar.
        let r = run("let c = int_to_char(0xD800); println(c);");
        assert!(
            !r.ok,
            "expected error for surrogate: ok={}, errors={:?}",
            r.ok, r.errors
        );
    }
}
