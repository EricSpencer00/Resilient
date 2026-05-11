//! RES-1140: complete the ASCII char-class predicate family.
//!
//! Seven new pure leaf builtins that round out the predicate surface
//! alongside the existing `is_ascii_alpha` / `is_ascii_digit` /
//! `is_ascii_alnum` (RES-459). Each delegates to the corresponding
//! `char::is_ascii_*` method and returns `true` iff *every* char in the
//! input string satisfies the predicate. Empty input returns `true`
//! (vacuously), matching the existing family's contract.
//!
//! | Builtin | Stdlib analog |
//! |---|---|
//! | `is_ascii(s)`             | `char::is_ascii` |
//! | `is_ascii_whitespace(s)`  | `char::is_ascii_whitespace` |
//! | `is_ascii_hexdigit(s)`    | `char::is_ascii_hexdigit` |
//! | `is_ascii_uppercase(s)`   | `char::is_ascii_uppercase` |
//! | `is_ascii_lowercase(s)`   | `char::is_ascii_lowercase` |
//! | `is_ascii_punctuation(s)` | `char::is_ascii_punctuation` |
//! | `is_ascii_control(s)`     | `char::is_ascii_control` |
//!
//! All seven share the same dispatch shape — a tiny local helper applies
//! the predicate to every char, returning the all-true conjunction. The
//! helper is duplicated from `lib.rs::ascii_all` to keep this module
//! self-contained; the call shape is small enough that the duplication
//! is cheaper than threading a `pub(crate)` helper out of the giant
//! `lib.rs` root.

use crate::{RResult, Value};

fn ascii_all<F: Fn(char) -> bool>(name: &str, args: &[Value], pred: F) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::Bool(s.chars().all(pred))),
        [other] => Err(format!("{}: expected string, got {}", name, other)),
        _ => Err(format!("{}: expected 1 argument, got {}", name, args.len())),
    }
}

/// `is_ascii(s) -> Bool` — true iff every char is in the ASCII range
/// (`0x00..=0x7F`). Empty string → true. Mirrors `char::is_ascii`.
pub(crate) fn builtin_is_ascii(args: &[Value]) -> RResult<Value> {
    ascii_all("is_ascii", args, |c| c.is_ascii())
}

/// `is_ascii_whitespace(s) -> Bool` — true iff every char is ASCII
/// whitespace (space, tab, newline, carriage return, form feed).
pub(crate) fn builtin_is_ascii_whitespace(args: &[Value]) -> RResult<Value> {
    ascii_all("is_ascii_whitespace", args, |c| c.is_ascii_whitespace())
}

/// `is_ascii_hexdigit(s) -> Bool` — true iff every char is in
/// `0..=9 | a..=f | A..=F`.
pub(crate) fn builtin_is_ascii_hexdigit(args: &[Value]) -> RResult<Value> {
    ascii_all("is_ascii_hexdigit", args, |c| c.is_ascii_hexdigit())
}

/// `is_ascii_uppercase(s) -> Bool` — true iff every char is in `A..=Z`.
pub(crate) fn builtin_is_ascii_uppercase(args: &[Value]) -> RResult<Value> {
    ascii_all("is_ascii_uppercase", args, |c| c.is_ascii_uppercase())
}

/// `is_ascii_lowercase(s) -> Bool` — true iff every char is in `a..=z`.
pub(crate) fn builtin_is_ascii_lowercase(args: &[Value]) -> RResult<Value> {
    ascii_all("is_ascii_lowercase", args, |c| c.is_ascii_lowercase())
}

/// `is_ascii_punctuation(s) -> Bool` — true iff every char is an ASCII
/// punctuation character (the 32 characters in `!"#$%&'()*+,-./:;<=>?@[\\]^_\`{|}~`).
pub(crate) fn builtin_is_ascii_punctuation(args: &[Value]) -> RResult<Value> {
    ascii_all("is_ascii_punctuation", args, |c| c.is_ascii_punctuation())
}

/// `is_ascii_control(s) -> Bool` — true iff every char is an ASCII
/// control character (`0x00..=0x1F` or `0x7F`).
pub(crate) fn builtin_is_ascii_control(args: &[Value]) -> RResult<Value> {
    ascii_all("is_ascii_control", args, |c| c.is_ascii_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(f: fn(&[Value]) -> RResult<Value>, s: &str) -> bool {
        match f(&[Value::String(s.to_string())]).unwrap() {
            Value::Bool(v) => v,
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn is_ascii_basic() {
        assert!(b(builtin_is_ascii, ""));
        assert!(b(builtin_is_ascii, "hello"));
        assert!(b(builtin_is_ascii, "Hello, World! 123"));
        assert!(b(builtin_is_ascii, "\x00\x7F"));
        // Non-ASCII (UTF-8) → false
        assert!(!b(builtin_is_ascii, "café"));
        assert!(!b(builtin_is_ascii, "日本"));
        assert!(!b(builtin_is_ascii, "\u{0080}"));
    }

    #[test]
    fn is_ascii_whitespace_basic() {
        assert!(b(builtin_is_ascii_whitespace, ""));
        assert!(b(builtin_is_ascii_whitespace, " "));
        assert!(b(builtin_is_ascii_whitespace, "\t"));
        assert!(b(builtin_is_ascii_whitespace, "\n"));
        assert!(b(builtin_is_ascii_whitespace, "\r"));
        assert!(b(builtin_is_ascii_whitespace, "\x0C")); // form feed
        assert!(b(builtin_is_ascii_whitespace, " \t\n\r"));
        assert!(!b(builtin_is_ascii_whitespace, "a"));
        assert!(!b(builtin_is_ascii_whitespace, " a "));
        assert!(!b(builtin_is_ascii_whitespace, "\u{00A0}")); // non-breaking space is NOT ASCII whitespace
    }

    #[test]
    fn is_ascii_hexdigit_basic() {
        assert!(b(builtin_is_ascii_hexdigit, ""));
        assert!(b(builtin_is_ascii_hexdigit, "0123456789"));
        assert!(b(builtin_is_ascii_hexdigit, "abcdef"));
        assert!(b(builtin_is_ascii_hexdigit, "ABCDEF"));
        assert!(b(builtin_is_ascii_hexdigit, "DeadBeef"));
        assert!(!b(builtin_is_ascii_hexdigit, "g"));
        assert!(!b(builtin_is_ascii_hexdigit, "0x123"));
        assert!(!b(builtin_is_ascii_hexdigit, "12 34"));
    }

    #[test]
    fn is_ascii_uppercase_basic() {
        assert!(b(builtin_is_ascii_uppercase, ""));
        assert!(b(builtin_is_ascii_uppercase, "ABC"));
        assert!(b(builtin_is_ascii_uppercase, "HELLO"));
        assert!(!b(builtin_is_ascii_uppercase, "Hello"));
        assert!(!b(builtin_is_ascii_uppercase, "abc"));
        assert!(!b(builtin_is_ascii_uppercase, "A1B"));
        assert!(!b(builtin_is_ascii_uppercase, "A B"));
    }

    #[test]
    fn is_ascii_lowercase_basic() {
        assert!(b(builtin_is_ascii_lowercase, ""));
        assert!(b(builtin_is_ascii_lowercase, "abc"));
        assert!(b(builtin_is_ascii_lowercase, "hello"));
        assert!(!b(builtin_is_ascii_lowercase, "Hello"));
        assert!(!b(builtin_is_ascii_lowercase, "ABC"));
        assert!(!b(builtin_is_ascii_lowercase, "a1b"));
    }

    #[test]
    fn is_ascii_punctuation_basic() {
        assert!(b(builtin_is_ascii_punctuation, ""));
        assert!(b(builtin_is_ascii_punctuation, "!@#"));
        assert!(b(builtin_is_ascii_punctuation, ".,;:"));
        assert!(b(builtin_is_ascii_punctuation, "()[]{}"));
        assert!(!b(builtin_is_ascii_punctuation, "a"));
        assert!(!b(builtin_is_ascii_punctuation, "1"));
        assert!(!b(builtin_is_ascii_punctuation, " "));
        assert!(!b(builtin_is_ascii_punctuation, "Hello!"));
    }

    #[test]
    fn is_ascii_control_basic() {
        assert!(b(builtin_is_ascii_control, ""));
        assert!(b(builtin_is_ascii_control, "\x00"));
        assert!(b(builtin_is_ascii_control, "\x1F"));
        assert!(b(builtin_is_ascii_control, "\x7F")); // DEL is control
        assert!(b(builtin_is_ascii_control, "\x00\x01\x02"));
        // Tab / newline / CR are also ASCII control chars
        assert!(b(builtin_is_ascii_control, "\t\n\r"));
        // Space (0x20) is NOT control
        assert!(!b(builtin_is_ascii_control, " "));
        assert!(!b(builtin_is_ascii_control, "a"));
        assert!(!b(builtin_is_ascii_control, "\x00a"));
    }

    #[test]
    fn rejects_non_string() {
        for f in [
            builtin_is_ascii,
            builtin_is_ascii_whitespace,
            builtin_is_ascii_hexdigit,
            builtin_is_ascii_uppercase,
            builtin_is_ascii_lowercase,
            builtin_is_ascii_punctuation,
            builtin_is_ascii_control,
        ] {
            let err = f(&[Value::Int(65)]).unwrap_err();
            assert!(err.contains("expected string"), "got {}", err);
        }
    }

    #[test]
    fn rejects_wrong_arity() {
        for f in [
            builtin_is_ascii,
            builtin_is_ascii_whitespace,
            builtin_is_ascii_hexdigit,
            builtin_is_ascii_uppercase,
            builtin_is_ascii_lowercase,
            builtin_is_ascii_punctuation,
            builtin_is_ascii_control,
        ] {
            let err = f(&[]).unwrap_err();
            assert!(err.contains("expected 1"), "got {}", err);
            let err = f(&[
                Value::String("a".to_string()),
                Value::String("b".to_string()),
            ])
            .unwrap_err();
            assert!(err.contains("expected 1"), "got {}", err);
        }
    }

    #[test]
    fn family_consistency_on_empty_string() {
        // Every "all chars satisfy P" predicate returns true on the
        // empty string vacuously.
        for f in [
            builtin_is_ascii,
            builtin_is_ascii_whitespace,
            builtin_is_ascii_hexdigit,
            builtin_is_ascii_uppercase,
            builtin_is_ascii_lowercase,
            builtin_is_ascii_punctuation,
            builtin_is_ascii_control,
        ] {
            assert!(b(f, ""));
        }
    }

    #[test]
    fn family_partitions_a_single_alpha_lower_char() {
        // 'a' is lowercase, alphabetic, ASCII — but not digit, not
        // upper, not punctuation, not control, not whitespace,
        // not hexdigit-only (well, 'a' IS a hexdigit).
        assert!(b(builtin_is_ascii, "a"));
        assert!(b(builtin_is_ascii_lowercase, "a"));
        assert!(b(builtin_is_ascii_hexdigit, "a"));
        assert!(!b(builtin_is_ascii_uppercase, "a"));
        assert!(!b(builtin_is_ascii_punctuation, "a"));
        assert!(!b(builtin_is_ascii_control, "a"));
        assert!(!b(builtin_is_ascii_whitespace, "a"));
    }
}
