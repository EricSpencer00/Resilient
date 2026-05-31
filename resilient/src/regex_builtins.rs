//! RES-2585: regular expression matching builtins.
//!
//! Provides `regex_match`, `regex_find`, `regex_find_all`,
//! `regex_captures`, `regex_replace`, and `regex_replace_all`.
//!
//! Compiled regexes are cached in a process-wide LRU so repeated
//! calls with the same pattern avoid re-compilation.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation)]

use crate::{Node, Value};
use regex::Regex;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

type RResult<T> = Result<T, String>;

// ---------------------------------------------------------------------------
// Compiled-regex cache
// ---------------------------------------------------------------------------

const CACHE_CAPACITY: usize = 64;

static REGEX_CACHE: LazyLock<RwLock<HashMap<String, Regex>>> =
    LazyLock::new(|| RwLock::new(HashMap::with_capacity(CACHE_CAPACITY)));

fn get_or_compile(pattern: &str) -> RResult<Regex> {
    if let Ok(cache) = REGEX_CACHE.read() {
        if let Some(re) = cache.get(pattern) {
            return Ok(re.clone());
        }
    }
    let re = Regex::new(pattern).map_err(|e| format!("invalid regex pattern: {e}"))?;
    if let Ok(mut cache) = REGEX_CACHE.write() {
        if cache.len() >= CACHE_CAPACITY {
            cache.clear();
        }
        cache.insert(pattern.to_string(), re.clone());
    }
    Ok(re)
}

// ---------------------------------------------------------------------------
// Builtins
// ---------------------------------------------------------------------------

pub(crate) fn builtin_regex_match(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(text), Value::String(pattern)] => {
            let re = get_or_compile(pattern)?;
            Ok(Value::Bool(re.is_match(text)))
        }
        [a, b] => Err(format!(
            "regex_match: expected (string, string), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "regex_match: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

pub(crate) fn builtin_regex_find(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(text), Value::String(pattern)] => {
            let re = get_or_compile(pattern)?;
            match re.find(text) {
                Some(m) => Ok(Value::Option(Some(Box::new(Value::String(
                    m.as_str().to_string(),
                ))))),
                None => Ok(Value::Option(None)),
            }
        }
        [a, b] => Err(format!(
            "regex_find: expected (string, string), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "regex_find: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

pub(crate) fn builtin_regex_find_all(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(text), Value::String(pattern)] => {
            let re = get_or_compile(pattern)?;
            let matches: Vec<Value> = re
                .find_iter(text)
                .map(|m| Value::String(m.as_str().to_string()))
                .collect();
            Ok(Value::Array(matches))
        }
        [a, b] => Err(format!(
            "regex_find_all: expected (string, string), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "regex_find_all: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

pub(crate) fn builtin_regex_captures(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(text), Value::String(pattern)] => {
            let re = get_or_compile(pattern)?;
            match re.captures(text) {
                Some(caps) => {
                    let groups: Vec<Value> = caps
                        .iter()
                        .map(|m| match m {
                            Some(m) => Value::String(m.as_str().to_string()),
                            None => Value::Void,
                        })
                        .collect();
                    Ok(Value::Option(Some(Box::new(Value::Array(groups)))))
                }
                None => Ok(Value::Option(None)),
            }
        }
        [a, b] => Err(format!(
            "regex_captures: expected (string, string), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "regex_captures: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

pub(crate) fn builtin_regex_replace(args: &[Value]) -> RResult<Value> {
    match args {
        [
            Value::String(text),
            Value::String(pattern),
            Value::String(replacement),
        ] => {
            let re = get_or_compile(pattern)?;
            Ok(Value::String(
                re.replace(text, replacement.as_str()).into_owned(),
            ))
        }
        [a, b, c] => Err(format!(
            "regex_replace: expected (string, string, string), got ({}, {}, {})",
            a, b, c
        )),
        _ => Err(format!(
            "regex_replace: expected 3 arguments, got {}",
            args.len()
        )),
    }
}

pub(crate) fn builtin_regex_replace_all(args: &[Value]) -> RResult<Value> {
    match args {
        [
            Value::String(text),
            Value::String(pattern),
            Value::String(replacement),
        ] => {
            let re = get_or_compile(pattern)?;
            Ok(Value::String(
                re.replace_all(text, replacement.as_str()).into_owned(),
            ))
        }
        [a, b, c] => Err(format!(
            "regex_replace_all: expected (string, string, string), got ({}, {}, {})",
            a, b, c
        )),
        _ => Err(format!(
            "regex_replace_all: expected 3 arguments, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// Feature pass (no-op — builtins are self-contained)
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

    fn assert_bool(v: &Value, expected: bool) {
        match v {
            Value::Bool(b) => assert_eq!(*b, expected),
            other => panic!("expected Bool({expected}), got {other:?}"),
        }
    }

    #[test]
    fn regex_match_basic() {
        assert_bool(
            &builtin_regex_match(&[s("hello123"), s("[a-z]+[0-9]+")]).unwrap(),
            true,
        );
        assert_bool(
            &builtin_regex_match(&[s("hello"), s("^[0-9]+$")]).unwrap(),
            false,
        );
    }

    #[test]
    fn regex_match_invalid_pattern_errors() {
        let result = builtin_regex_match(&[s("hello"), s("[invalid")]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid regex"));
    }

    #[test]
    fn regex_find_returns_first_match() {
        let result = builtin_regex_find(&[s("abc 123 def 456"), s("[0-9]+")]).unwrap();
        match result {
            Value::Option(Some(boxed)) => assert_str(&boxed, "123"),
            other => panic!("expected Some, got {other:?}"),
        }
    }

    #[test]
    fn regex_find_returns_none_on_no_match() {
        let result = builtin_regex_find(&[s("abcdef"), s("[0-9]+")]).unwrap();
        assert!(matches!(result, Value::Option(None)));
    }

    #[test]
    fn regex_find_all_returns_all_matches() {
        let result = builtin_regex_find_all(&[s("abc 123 def 456"), s("[0-9]+")]).unwrap();
        match result {
            Value::Array(items) => {
                assert_eq!(items.len(), 2);
                assert_str(&items[0], "123");
                assert_str(&items[1], "456");
            }
            other => panic!("expected Array, got {other:?}"),
        }
    }

    #[test]
    fn regex_find_all_empty_on_no_match() {
        let result = builtin_regex_find_all(&[s("abcdef"), s("[0-9]+")]).unwrap();
        match result {
            Value::Array(items) => assert!(items.is_empty()),
            other => panic!("expected empty Array, got {other:?}"),
        }
    }

    #[test]
    fn regex_captures_with_groups() {
        let result =
            builtin_regex_captures(&[s("2024-01-15"), s(r"(\d{4})-(\d{2})-(\d{2})")]).unwrap();
        match result {
            Value::Option(Some(boxed)) => match *boxed {
                Value::Array(ref groups) => {
                    assert_eq!(groups.len(), 4);
                    assert_str(&groups[0], "2024-01-15");
                    assert_str(&groups[1], "2024");
                    assert_str(&groups[2], "01");
                    assert_str(&groups[3], "15");
                }
                ref other => panic!("expected Array, got {other:?}"),
            },
            other => panic!("expected Some, got {other:?}"),
        }
    }

    #[test]
    fn regex_captures_returns_none_on_no_match() {
        let result = builtin_regex_captures(&[s("hello"), s(r"(\d+)")]).unwrap();
        assert!(matches!(result, Value::Option(None)));
    }

    #[test]
    fn regex_replace_first_occurrence() {
        let result = builtin_regex_replace(&[s("foo bar baz"), s(r"\s+"), s("_")]).unwrap();
        assert_str(&result, "foo_bar baz");
    }

    #[test]
    fn regex_replace_all_occurrences() {
        let result = builtin_regex_replace_all(&[s("foo bar baz"), s(r"\s+"), s("_")]).unwrap();
        assert_str(&result, "foo_bar_baz");
    }

    #[test]
    fn regex_replace_with_capture_groups() {
        let result =
            builtin_regex_replace_all(&[s("John Smith"), s(r"(\w+)\s(\w+)"), s("$2, $1")]).unwrap();
        assert_str(&result, "Smith, John");
    }

    #[test]
    fn regex_wrong_types_error() {
        assert!(builtin_regex_match(&[Value::Int(42), s("pat")]).is_err());
        assert!(builtin_regex_find(&[s("text"), Value::Bool(true)]).is_err());
        assert!(builtin_regex_replace(&[s("t"), s("p")]).is_err());
    }

    #[test]
    fn cache_reuse_same_pattern() {
        let _ = builtin_regex_match(&[s("test"), s("^t")]).unwrap();
        let _ = builtin_regex_match(&[s("test2"), s("^t")]).unwrap();
        let cache = REGEX_CACHE.read().unwrap();
        assert!(cache.contains_key("^t"));
    }

    #[test]
    fn end_to_end_regex_match() {
        let r = crate::run_program(
            r#"
let matched = regex_match("hello123", "[a-z]+[0-9]+")
println(matched)
let no_match = regex_match("hello", "^[0-9]+$")
println(no_match)
"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines, vec!["true", "false"]);
    }

    #[test]
    fn end_to_end_regex_find() {
        let r = crate::run_program(
            r#"
let result = regex_find("price: $42.99", "[0-9]+\\.[0-9]+")
match result {
    Some(s) => println(s),
    None => println("not found"),
}
"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert_eq!(r.stdout.trim(), "42.99");
    }

    #[test]
    fn end_to_end_regex_find_all() {
        let r = crate::run_program(
            r#"
let words = regex_find_all("hello world foo", "[a-z]+")
for w in words {
    println(w)
}
"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines, vec!["hello", "world", "foo"]);
    }

    #[test]
    fn end_to_end_regex_replace_all() {
        let r = crate::run_program(
            r#"
let result = regex_replace_all("foo  bar   baz", "\\s+", " ")
println(result)
"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert_eq!(r.stdout.trim(), "foo bar baz");
    }

    #[test]
    fn end_to_end_invalid_pattern_errors() {
        let r = crate::run_program(
            r#"
let result = regex_match("test", "[invalid")
println(result)
"#,
        );
        assert!(!r.ok, "should error on invalid regex pattern");
    }

    #[test]
    fn end_to_end_regex_captures() {
        let r = crate::run_program(
            r#"
let caps = regex_captures("John Smith", "(\\w+)\\s(\\w+)")
match caps {
    Some(groups) => {
        println(groups[1])
        println(groups[2])
    },
    None => println("no match"),
}
"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines, vec!["John", "Smith"]);
    }
}
