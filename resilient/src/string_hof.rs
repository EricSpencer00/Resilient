//! RES-2649: Higher-order string operations with arbitrary callbacks.
//!
//! * `string_map_chars(s, fn)` — apply `fn(char) -> string` to each character
//!   and concatenate the results.
//! * `string_filter_by(s, fn)` — keep only characters for which `fn(char) -> bool`
//!   is true; returns new string.
//! * `string_fold(s, init, fn)` — reduce string characters left-to-right:
//!   `fn(acc, char) -> acc`.
//! * `string_for_each_char(s, fn)` — call `fn(char)` for each character for
//!   side effects; returns Void.

use crate::{Interpreter, Value};

type RResult<T> = Result<T, String>;

/// `string_map_chars(s, fn) -> string`
///
/// Calls `fn(c)` for every character `c` of `s` (each passed as a 1-char
/// string). `fn` must return a string. The returned strings are concatenated.
/// This allows characters to be transformed, doubled, deleted (return `""`),
/// or replaced with multi-character sequences.
///
/// ```text
/// let upper = string_map_chars("hello", fn(string c) -> string { to_upper(c) });
/// // upper == "HELLO"
/// ```
pub(crate) fn builtin_string_map_chars(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    // RES-1928: borrow `s` and `f` directly from `args` — `apply_function`
    // takes `func: &Value`, and the loop body only iterates `s.chars()` /
    // reads `s.len()`, both of which accept `&str`. The legacy
    // `s.clone()` / `f.clone()` pair was pure overhead (one String alloc
    // + one Function Rc bump per call).
    let (s, f) = match args {
        [Value::String(s), f] => (s.as_str(), f),
        [a, _] => {
            return Err(format!(
                "string_map_chars: first argument must be a string, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "string_map_chars: expected 2 arguments (string, fn), got {}",
                args.len()
            ));
        }
    };

    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let c_str = Value::String(ch.to_string());
        match interp.apply_function(f, vec![c_str])? {
            Value::String(piece) => out.push_str(&piece),
            other => {
                return Err(format!(
                    "string_map_chars: callback must return a string, got {other}"
                ));
            }
        }
    }
    Ok(Value::String(out))
}

/// `string_filter_by(s, fn) -> string`
///
/// Returns a new string containing only those characters of `s` for which
/// `fn(char) -> bool` returns true.
///
/// ```text
/// let digits_only = string_filter_by("a1b2c3", fn(string c) -> bool {
///     return is_ascii_digit(c);
/// });
/// // digits_only == "123"
/// ```
pub(crate) fn builtin_string_filter_by(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    // RES-1928: borrow `s` and `f` directly from `args`. See
    // `builtin_string_map_chars` above for rationale.
    let (s, f) = match args {
        [Value::String(s), f] => (s.as_str(), f),
        [a, _] => {
            return Err(format!(
                "string_filter_by: first argument must be a string, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "string_filter_by: expected 2 arguments (string, fn), got {}",
                args.len()
            ));
        }
    };

    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let c_str = Value::String(ch.to_string());
        match interp.apply_function(f, vec![c_str])? {
            Value::Bool(true) => out.push(ch),
            Value::Bool(false) => {}
            other => {
                return Err(format!(
                    "string_filter_by: callback must return bool, got {other}"
                ));
            }
        }
    }
    Ok(Value::String(out))
}

/// `string_fold(s, init, fn) -> value`
///
/// Reduces the characters of `s` left-to-right using `fn(acc, char) -> acc`.
/// `init` is the initial accumulator. The character is passed as a 1-char string.
///
/// ```text
/// let count = string_fold("hello world", 0, fn(int acc, string c) -> int {
///     if c == " " { return acc + 1; }
///     return acc;
/// });
/// // count == 1  (one space)
/// ```
pub(crate) fn builtin_string_fold(interp: &mut Interpreter, args: &[Value]) -> RResult<Value> {
    // RES-1928: `s` and `f` are borrows; `init` still needs `.clone()`
    // because it seeds the mutable `acc` accumulator that gets rebound
    // each loop iteration.
    let (s, init, f) = match args {
        [Value::String(s), init, f] => (s.as_str(), init.clone(), f),
        [a, _, _] => {
            return Err(format!(
                "string_fold: first argument must be a string, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "string_fold: expected 3 arguments (string, init, fn), got {}",
                args.len()
            ));
        }
    };

    let mut acc = init;
    for ch in s.chars() {
        let c_str = Value::String(ch.to_string());
        acc = interp.apply_function(f, vec![acc, c_str])?;
    }
    Ok(acc)
}

/// `string_for_each_char(s, fn) -> Void`
///
/// Calls `fn(char)` for each character of `s` for side effects. The return
/// value of `fn` is discarded. Returns Void.
///
/// ```text
/// string_for_each_char("abc", fn(string c) -> Void { println(c); });
/// // prints a, b, c on separate lines
/// ```
pub(crate) fn builtin_string_for_each_char(
    interp: &mut Interpreter,
    args: &[Value],
) -> RResult<Value> {
    // RES-1928: borrow `s` and `f` directly from `args`.
    let (s, f) = match args {
        [Value::String(s), f] => (s.as_str(), f),
        [a, _] => {
            return Err(format!(
                "string_for_each_char: first argument must be a string, got {a}"
            ));
        }
        _ => {
            return Err(format!(
                "string_for_each_char: expected 2 arguments (string, fn), got {}",
                args.len()
            ));
        }
    };

    for ch in s.chars() {
        let c_str = Value::String(ch.to_string());
        interp.apply_function(f, vec![c_str])?;
    }
    Ok(Value::Void)
}

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── string_map_chars ──────────────────────────────────────────────────────

    #[test]
    fn map_chars_to_upper() {
        let r = run(
            r#"let up = string_map_chars("hello", fn(string c) -> string { return to_upper(c); });
println(up);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("HELLO"), "stdout: {}", r.stdout);
    }

    #[test]
    fn map_chars_double_each() {
        let r = run(
            r#"let doubled = string_map_chars("ab", fn(string c) -> string { return c + c; });
println(doubled);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("aabb"), "stdout: {}", r.stdout);
    }

    #[test]
    fn map_chars_delete_vowels() {
        let r = run(
            r#"let no_vowels = string_map_chars("hello", fn(string c) -> string {
    if c == "a" || c == "e" || c == "i" || c == "o" || c == "u" { return ""; }
    return c;
});
println(no_vowels);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("hll"), "stdout: {}", r.stdout);
    }

    #[test]
    fn map_chars_empty_string() {
        let r = run(
            r#"let r = string_map_chars("", fn(string c) -> string { return c; });
println(len(r));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'));
    }

    #[test]
    fn map_chars_rejects_non_string_return() {
        let r = run(
            r#"let r = string_map_chars("a", fn(string c) -> int { return 1; });
println(r);"#,
        );
        assert!(!r.ok, "expected error for int return");
    }

    // ── string_filter_by ──────────────────────────────────────────────────────

    #[test]
    fn filter_by_keeps_digits() {
        let r = run(
            r#"let digits = string_filter_by("a1b2c3", fn(string c) -> bool {
    return is_ascii_digit(c);
});
println(digits);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("123"), "stdout: {}", r.stdout);
    }

    #[test]
    fn filter_by_none_match() {
        let r = run(
            r#"let r = string_filter_by("aaa", fn(string c) -> bool { return false; });
println(len(r));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'));
    }

    #[test]
    fn filter_by_all_match() {
        let r = run(
            r#"let r = string_filter_by("xyz", fn(string c) -> bool { return true; });
println(r);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("xyz"), "stdout: {}", r.stdout);
    }

    // ── string_fold ───────────────────────────────────────────────────────────

    #[test]
    fn fold_count_spaces() {
        let r = run(
            r#"let n = string_fold("hello world foo", 0, fn(int acc, string c) -> int {
    if c == " " { return acc + 1; }
    return acc;
});
println(n);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('2'), "stdout: {}", r.stdout);
    }

    #[test]
    fn fold_empty_string_returns_init() {
        let r = run(
            r#"let r = string_fold("", 42, fn(int acc, string c) -> int { return acc + 1; });
println(r);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("42"), "stdout: {}", r.stdout);
    }

    #[test]
    fn fold_build_string_accumulator() {
        let r = run(
            r#"let rev = string_fold("abc", "", fn(string acc, string c) -> string { return c + acc; });
println(rev);"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("cba"), "stdout: {}", r.stdout);
    }

    // ── string_for_each_char ──────────────────────────────────────────────────

    #[test]
    fn for_each_char_visits_all() {
        let r = run(r#"let count = [0];
string_for_each_char("hello", fn(string c) -> Void { count[0] = count[0] + 1; });
println(count[0]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('5'), "stdout: {}", r.stdout);
    }

    #[test]
    fn for_each_char_empty_string_no_calls() {
        let r = run(r#"let count = [0];
string_for_each_char("", fn(string c) -> Void { count[0] = count[0] + 1; });
println(count[0]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'));
    }
}
