//! RES-1172: small string + array gaps.
//!
//! Four pure leaf builtins:
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `string_split_once(s, sep)`  | `(String, String) -> Array` | `[before, after]` on first match, else `[s]` |
//! | `string_rsplit_once(s, sep)` | `(String, String) -> Array` | Same but from the right |
//! | `string_from_chars(arr)`     | `(Array) -> String`         | Inverse of `string_chars` |
//! | `array_is_empty(arr)`        | `(Array) -> Bool`           | True iff zero elements |

use crate::{RResult, Value};

fn split_array_2(before: String, after: String) -> Value {
    Value::Array(vec![Value::String(before), Value::String(after)])
}

fn single_array(s: String) -> Value {
    Value::Array(vec![Value::String(s)])
}

/// `string_split_once(s, sep) -> Array` — split on the first
/// occurrence of `sep`. Returns `[before, after]` if `sep` is found,
/// or `[s]` if not. Empty `sep` is a typed error.
pub(crate) fn builtin_string_split_once(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s), Value::String(sep)] => {
            if sep.is_empty() {
                return Err("string_split_once: separator must not be empty".to_string());
            }
            match s.split_once(sep.as_str()) {
                Some((a, b)) => Ok(split_array_2(a.to_string(), b.to_string())),
                None => Ok(single_array(s.clone())),
            }
        }
        [a, b] => Err(format!(
            "string_split_once: expected (string, string), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "string_split_once: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `string_rsplit_once(s, sep) -> Array` — same as `string_split_once`
/// but searches from the right. Useful for "path/file.ext" → (path, file).
pub(crate) fn builtin_string_rsplit_once(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s), Value::String(sep)] => {
            if sep.is_empty() {
                return Err("string_rsplit_once: separator must not be empty".to_string());
            }
            match s.rsplit_once(sep.as_str()) {
                Some((a, b)) => Ok(split_array_2(a.to_string(), b.to_string())),
                None => Ok(single_array(s.clone())),
            }
        }
        [a, b] => Err(format!(
            "string_rsplit_once: expected (string, string), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "string_rsplit_once: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `string_from_chars(arr) -> String` — join an array of single-char
/// strings into one string. Inverse of `string_chars`. Each element
/// must be a String of exactly one Unicode scalar; multi-char strings
/// and non-String elements are typed errors.
pub(crate) fn builtin_string_from_chars(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            let mut out = String::new();
            for v in items {
                match v {
                    Value::String(s) => {
                        let mut chars = s.chars();
                        let first = chars.next().ok_or_else(|| {
                            "string_from_chars: empty string at array element".to_string()
                        })?;
                        if chars.next().is_some() {
                            return Err(format!(
                                "string_from_chars: array element must be a single char, got {:?}",
                                s
                            ));
                        }
                        out.push(first);
                    }
                    other => {
                        return Err(format!(
                            "string_from_chars: array element must be String, got {}",
                            other
                        ));
                    }
                }
            }
            Ok(Value::String(out))
        }
        [other] => Err(format!("string_from_chars: expected array, got {}", other)),
        _ => Err(format!(
            "string_from_chars: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `array_is_empty(arr) -> Bool` — true iff `arr` has zero elements.
pub(crate) fn builtin_array_is_empty(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => Ok(Value::Bool(items.is_empty())),
        [other] => Err(format!("array_is_empty: expected array, got {}", other)),
        _ => Err(format!(
            "array_is_empty: expected 1 argument, got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Value {
        Value::String(x.to_string())
    }

    fn as_string(v: Value) -> String {
        match v {
            Value::String(s) => s,
            other => panic!("expected String, got {:?}", other),
        }
    }

    fn as_bool(v: Value) -> bool {
        match v {
            Value::Bool(b) => b,
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    fn as_string_vec(v: Value) -> Vec<String> {
        match v {
            Value::Array(items) => items
                .into_iter()
                .map(|x| match x {
                    Value::String(s) => s,
                    other => panic!("expected String, got {:?}", other),
                })
                .collect(),
            other => panic!("expected Array, got {:?}", other),
        }
    }

    // --- split_once ---

    #[test]
    fn split_once_basic() {
        let r = builtin_string_split_once(&[s("key=value"), s("=")]).unwrap();
        assert_eq!(as_string_vec(r), vec!["key", "value"]);
    }

    #[test]
    fn split_once_first_match_only() {
        // "a=b=c" splits at the FIRST '='.
        let r = builtin_string_split_once(&[s("a=b=c"), s("=")]).unwrap();
        assert_eq!(as_string_vec(r), vec!["a", "b=c"]);
    }

    #[test]
    fn split_once_no_match_returns_single() {
        let r = builtin_string_split_once(&[s("hello"), s("=")]).unwrap();
        assert_eq!(as_string_vec(r), vec!["hello"]);
    }

    #[test]
    fn split_once_empty_string_no_match() {
        let r = builtin_string_split_once(&[s(""), s("=")]).unwrap();
        assert_eq!(as_string_vec(r), vec![""]);
    }

    #[test]
    fn split_once_multichar_separator() {
        let r = builtin_string_split_once(&[s("foo--bar--baz"), s("--")]).unwrap();
        assert_eq!(as_string_vec(r), vec!["foo", "bar--baz"]);
    }

    #[test]
    fn split_once_rejects_empty_separator() {
        let err = builtin_string_split_once(&[s("hello"), s("")]).unwrap_err();
        assert!(err.contains("must not be empty"));
    }

    // --- rsplit_once ---

    #[test]
    fn rsplit_once_path_style() {
        let r = builtin_string_rsplit_once(&[s("path/to/file.ext"), s("/")]).unwrap();
        assert_eq!(as_string_vec(r), vec!["path/to", "file.ext"]);
    }

    #[test]
    fn rsplit_once_last_match() {
        let r = builtin_string_rsplit_once(&[s("a=b=c"), s("=")]).unwrap();
        assert_eq!(as_string_vec(r), vec!["a=b", "c"]);
    }

    #[test]
    fn rsplit_once_no_match() {
        let r = builtin_string_rsplit_once(&[s("hello"), s("=")]).unwrap();
        assert_eq!(as_string_vec(r), vec!["hello"]);
    }

    #[test]
    fn rsplit_once_rejects_empty_separator() {
        let err = builtin_string_rsplit_once(&[s("hello"), s("")]).unwrap_err();
        assert!(err.contains("must not be empty"));
    }

    // --- string_from_chars ---

    #[test]
    fn from_chars_basic() {
        let arr = Value::Array(vec![s("h"), s("e"), s("l"), s("l"), s("o")]);
        let r = builtin_string_from_chars(&[arr]).unwrap();
        assert_eq!(as_string(r), "hello");
    }

    #[test]
    fn from_chars_empty_array_is_empty_string() {
        let arr = Value::Array(vec![]);
        let r = builtin_string_from_chars(&[arr]).unwrap();
        assert_eq!(as_string(r), "");
    }

    #[test]
    fn from_chars_multi_byte_codepoints() {
        // 🌟 is a 4-byte UTF-8 codepoint, but a single Unicode scalar.
        let arr = Value::Array(vec![s("🌟"), s("h"), s("i")]);
        let r = builtin_string_from_chars(&[arr]).unwrap();
        assert_eq!(as_string(r), "🌟hi");
    }

    #[test]
    fn from_chars_rejects_multi_char_element() {
        let arr = Value::Array(vec![s("h"), s("el"), s("lo")]);
        let err = builtin_string_from_chars(&[arr]).unwrap_err();
        assert!(err.contains("single char"));
    }

    #[test]
    fn from_chars_rejects_empty_element() {
        let arr = Value::Array(vec![s("h"), s("")]);
        let err = builtin_string_from_chars(&[arr]).unwrap_err();
        assert!(err.contains("empty string"));
    }

    #[test]
    fn from_chars_rejects_non_string_element() {
        let arr = Value::Array(vec![s("h"), Value::Int(105)]);
        let err = builtin_string_from_chars(&[arr]).unwrap_err();
        assert!(err.contains("must be String"));
    }

    // --- array_is_empty ---

    #[test]
    fn array_is_empty_true_for_empty() {
        let r = builtin_array_is_empty(&[Value::Array(vec![])]).unwrap();
        assert!(as_bool(r));
    }

    #[test]
    fn array_is_empty_false_for_non_empty() {
        let r = builtin_array_is_empty(&[Value::Array(vec![Value::Int(1)])]).unwrap();
        assert!(!as_bool(r));
    }

    #[test]
    fn array_is_empty_rejects_non_array() {
        let err = builtin_array_is_empty(&[Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected array"));
    }

    // --- general ---

    #[test]
    fn arity_diagnostics_consistent() {
        let err = builtin_string_split_once(&[]).unwrap_err();
        assert!(err.contains("expected 2"));
        let err = builtin_string_rsplit_once(&[]).unwrap_err();
        assert!(err.contains("expected 2"));
        let err = builtin_string_from_chars(&[]).unwrap_err();
        assert!(err.contains("expected 1"));
        let err = builtin_array_is_empty(&[]).unwrap_err();
        assert!(err.contains("expected 1"));
    }

    #[test]
    fn round_trip_chars_inverse() {
        // For an ASCII string: chars(s) = arr; from_chars(arr) = s.
        let original = "abcdef";
        let chars: Vec<Value> = original.chars().map(|c| s(&c.to_string())).collect();
        let r = builtin_string_from_chars(&[Value::Array(chars)]).unwrap();
        assert_eq!(as_string(r), original);
    }
}
