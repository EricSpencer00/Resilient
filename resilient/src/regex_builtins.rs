//! RES-2585: Regex matching builtins.
//!
//! Provides regex operations backed by the `regex` crate (std-only — the
//! embedded runtime has no regex support). All functions compile the pattern
//! on each call; for hot paths the caller should cache the result in a let.
//!
//! API:
//!   regex_match(text, pattern)           → bool
//!   regex_find(text, pattern)            → Option<string>
//!   regex_find_all(text, pattern)        → [string]
//!   regex_captures(text, pattern)        → Option<[string]>
//!   regex_replace(text, pattern, repl)   → string  — replaces first match
//!   regex_replace_all(text, pattern, repl) → string — replaces all matches

use crate::Value;
use regex::Regex;

type RResult<T> = Result<T, String>;

fn compile(pattern: &str) -> RResult<Regex> {
    Regex::new(pattern).map_err(|e| format!("regex_compile: invalid pattern {:?}: {}", pattern, e))
}

/// `regex_match(text, pattern) → bool`
pub(crate) fn builtin_regex_match(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(text), Value::String(pattern)] => {
            let re = compile(pattern)?;
            Ok(Value::Bool(re.is_match(text)))
        }
        [_, _] => Err("regex_match: expected (string, string)".to_string()),
        _ => Err(format!(
            "regex_match: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `regex_find(text, pattern) → Option<string>` — first match.
pub(crate) fn builtin_regex_find(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(text), Value::String(pattern)] => {
            let re = compile(pattern)?;
            Ok(Value::Option(
                re.find(text)
                    .map(|m| Box::new(Value::String(m.as_str().to_string()))),
            ))
        }
        [_, _] => Err("regex_find: expected (string, string)".to_string()),
        _ => Err(format!(
            "regex_find: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `regex_find_all(text, pattern) → [string]` — all non-overlapping matches.
pub(crate) fn builtin_regex_find_all(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(text), Value::String(pattern)] => {
            let re = compile(pattern)?;
            let matches: Vec<Value> = re
                .find_iter(text)
                .map(|m| Value::String(m.as_str().to_string()))
                .collect();
            Ok(Value::Array(matches))
        }
        [_, _] => Err("regex_find_all: expected (string, string)".to_string()),
        _ => Err(format!(
            "regex_find_all: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `regex_captures(text, pattern) → Option<[string]>` — capture groups (index 0 = whole match).
pub(crate) fn builtin_regex_captures(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(text), Value::String(pattern)] => {
            let re = compile(pattern)?;
            Ok(Value::Option(re.captures(text).map(|caps| {
                let groups: Vec<Value> = caps
                    .iter()
                    .map(|m| match m {
                        Some(m) => Value::String(m.as_str().to_string()),
                        None => Value::Option(None),
                    })
                    .collect();
                Box::new(Value::Array(groups))
            })))
        }
        [_, _] => Err("regex_captures: expected (string, string)".to_string()),
        _ => Err(format!(
            "regex_captures: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `regex_replace(text, pattern, replacement) → string` — replace first match.
pub(crate) fn builtin_regex_replace(args: &[Value]) -> RResult<Value> {
    match args {
        [
            Value::String(text),
            Value::String(pattern),
            Value::String(repl),
        ] => {
            let re = compile(pattern)?;
            Ok(Value::String(re.replace(text, repl.as_str()).into_owned()))
        }
        [_, _, _] => Err("regex_replace: expected (string, string, string)".to_string()),
        _ => Err(format!(
            "regex_replace: expected 3 arguments, got {}",
            args.len()
        )),
    }
}

/// `regex_replace_all(text, pattern, replacement) → string` — replace all matches.
pub(crate) fn builtin_regex_replace_all(args: &[Value]) -> RResult<Value> {
    match args {
        [
            Value::String(text),
            Value::String(pattern),
            Value::String(repl),
        ] => {
            let re = compile(pattern)?;
            Ok(Value::String(
                re.replace_all(text, repl.as_str()).into_owned(),
            ))
        }
        [_, _, _] => Err("regex_replace_all: expected (string, string, string)".to_string()),
        _ => Err(format!(
            "regex_replace_all: expected 3 arguments, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> String {
        let r = run_program(src);
        assert!(r.ok, "program failed: {:?}", r.errors);
        r.stdout
    }

    #[test]
    fn regex_match_true_false() {
        let out = run(r#"
println(to_string(regex_match("hello123", "[a-z]+[0-9]+")));
println(to_string(regex_match("hello", "[0-9]+")));
"#);
        assert!(out.contains("true"), "got: {out:?}");
        assert!(out.contains("false"), "got: {out:?}");
    }

    #[test]
    fn regex_find_some_and_none() {
        let out = run(r#"
let found = regex_find("hello world", "[a-z]+");
let v = match found { Some(s) => s, None => "none" };
println(v);
let not_found = regex_find("hello", "[0-9]+");
let v2 = match not_found { Some(s) => s, None => "none" };
println(v2);
"#);
        assert!(out.contains("hello"), "got: {out:?}");
        assert!(out.contains("none"), "got: {out:?}");
    }

    #[test]
    fn regex_find_all_returns_array() {
        let out = run(r#"
let matches = regex_find_all("cat bat hat", "[a-z]at");
println(to_string(len(matches)));
"#);
        assert!(out.contains("3"), "got: {out:?}");
    }

    #[test]
    fn regex_replace_first() {
        let out = run(r#"
let result = regex_replace("foo bar foo", "foo", "baz");
println(result);
"#);
        assert!(out.contains("baz bar foo"), "got: {out:?}");
    }

    #[test]
    fn regex_replace_all_replaces_all() {
        let out = run(r#"
let result = regex_replace_all("foo bar foo", "foo", "baz");
println(result);
"#);
        assert!(out.contains("baz bar baz"), "got: {out:?}");
    }

    #[test]
    fn regex_captures_groups() {
        let out = run(r#"
let caps = regex_captures("hello world", "(hello) (world)");
let arr = match caps { Some(a) => a, None => [] };
println(to_string(len(arr)));
"#);
        assert!(out.contains("3"), "got: {out:?}");
    }
}
