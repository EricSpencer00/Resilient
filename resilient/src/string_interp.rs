//! RES-221: string interpolation — `"text {expr} more"`.
//!
//! Syntax: any double-quoted string containing `{...}` segments is
//! an interpolated string. The expression inside `{...}` is evaluated
//! at runtime and spliced in as text.
//!
//! - `\{` produces a literal `{` (escape, no interpolation).
//! - `{{` is a parse-time error; the escape form `\{` should be used.
//! - Nesting `{...{...}...}` is not supported; the first `}` closes.
//!
//! This module owns:
//! - [`StringPart`]: the parts of a parsed interpolated string.
//! - [`parse_parts`]: splits a raw string value into literal/expr parts.
//! - [`eval_interp`]: evaluates an `InterpolatedString` node at runtime.
//! - [`check`]: top-level pass (no-op; parse-time errors are sufficient).

use crate::{Interpreter, Lexer, Node, Parser, RResult, Value};

// ---------- AST helpers ----------

/// One segment of an interpolated string.
#[derive(Debug, Clone)]
pub enum StringPart {
    /// A run of literal characters (no interpolation needed).
    Literal(String),
    /// A Resilient expression to be evaluated and stringified.
    Expr(Box<Node>),
}

// ---------- Parser ----------

/// Walk `raw` (the string literal's inner text, without surrounding
/// quotes) and split it into [`StringPart`]s.
///
/// Returns `None` when no `{` is present — callers may keep the
/// string as a plain `Node::StringLiteral` in that case.
/// Returns `Err` when a `{` is found but the sub-expression cannot be
/// parsed or the syntax is otherwise malformed.
pub(crate) fn parse_parts(raw: &str) -> Result<Option<Vec<StringPart>>, String> {
    if !raw.contains('{') {
        return Ok(None);
    }

    let mut parts: Vec<StringPart> = Vec::new();
    let mut literal_buf = String::new();
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            // `\{` — escaped brace, emit literal `{`.
            '\\' if chars.peek() == Some(&'{') => {
                chars.next();
                literal_buf.push('{');
            }
            '{' => {
                // Opening brace — everything up to `}` is a sub-expression.
                if chars.peek() == Some(&'{') {
                    return Err("nested braces are not allowed in string interpolation; \
                         escape `{` as `\\{`"
                        .to_string());
                }

                // Flush the pending literal fragment.
                if !literal_buf.is_empty() {
                    parts.push(StringPart::Literal(literal_buf.clone()));
                    literal_buf.clear();
                }

                // Collect source up to the matching `}`.
                let mut expr_src = String::new();
                let mut found_close = false;
                for inner in chars.by_ref() {
                    if inner == '}' {
                        found_close = true;
                        break;
                    }
                    expr_src.push(inner);
                }
                if !found_close {
                    return Err("unterminated interpolation: `{` has no matching `}`".to_string());
                }
                if expr_src.trim().is_empty() {
                    return Err("empty interpolation `{}` is not allowed".to_string());
                }

                // Parse the sub-expression using the main parser.
                let lexer = Lexer::new(expr_src.clone());
                let mut sub_parser = Parser::new(lexer);
                let expr_node = match sub_parser.parse_expression(0) {
                    Some(n) if sub_parser.errors.is_empty() => n,
                    Some(_) => {
                        return Err(format!(
                            "syntax error in string interpolation `{{{}}}`{}",
                            expr_src,
                            if let Some(e) = sub_parser.errors.first() {
                                format!(": {}", e)
                            } else {
                                String::new()
                            }
                        ));
                    }
                    None => {
                        return Err(format!(
                            "could not parse expression in string interpolation `{{{}}}`",
                            expr_src
                        ));
                    }
                };

                parts.push(StringPart::Expr(Box::new(expr_node)));
            }
            other => literal_buf.push(other),
        }
    }

    // Flush any trailing literal.
    if !literal_buf.is_empty() {
        parts.push(StringPart::Literal(literal_buf));
    }

    Ok(Some(parts))
}

// ---------- Evaluator ----------

/// Evaluate an `InterpolatedString` node. Each part is either copied
/// directly into the output string or evaluated and converted to its
/// string representation.
pub(crate) fn eval_interp(interp: &mut Interpreter, parts: &[StringPart]) -> RResult<Value> {
    let mut out = String::new();
    for part in parts {
        match part {
            StringPart::Literal(s) => out.push_str(s),
            StringPart::Expr(expr) => {
                let val = interp.eval(expr)?;
                out.push_str(&value_to_string(val));
            }
        }
    }
    Ok(Value::String(out))
}

/// Convert a [`Value`] to its plain text form for string interpolation.
///
/// Strings yield their raw contents (no surrounding quotes).
/// Integers, floats, and booleans convert via `Display`.
/// All other values (arrays, structs, maps, etc.) use the `Display`
/// implementation of `Value`, which matches what `println` would show.
fn value_to_string(v: Value) -> String {
    match v {
        Value::String(s) => s,
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        other => format!("{}", other),
    }
}

// ---------- Type-check pass ----------

/// RES-221: top-level type-check pass for interpolated strings.
///
/// Parse-time validation (unterminated braces, empty interpolations,
/// syntax errors in sub-expressions) is already handled in
/// [`parse_parts`], so this pass is a no-op today. The extension-pass
/// slot is kept for future work (e.g. type-checking interpolated
/// sub-expressions against `String`-compatible types).
pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn interp_eval(src: &str) -> String {
        let lexer = crate::Lexer::new(src.to_string());
        let mut parser = crate::Parser::new(lexer);
        let program = parser.parse_program();
        assert!(
            parser.errors.is_empty(),
            "parse errors: {:?}",
            parser.errors
        );
        let mut interp = crate::Interpreter::new();
        match interp.eval(&program) {
            Ok(Value::String(s)) => s,
            Ok(other) => format!("{}", other),
            Err(e) => panic!("eval error: {}", e),
        }
    }

    #[test]
    fn no_braces_returns_none() {
        assert!(parse_parts("hello world").unwrap().is_none());
    }

    #[test]
    fn escaped_brace_is_literal() {
        let parts = parse_parts("a\\{b").unwrap().expect("should have parts");
        assert_eq!(parts.len(), 1);
        assert!(matches!(&parts[0], StringPart::Literal(s) if s == "a{b"));
    }

    #[test]
    fn nested_braces_error() {
        assert!(parse_parts("{{x}}").is_err());
    }

    #[test]
    fn empty_interpolation_error() {
        assert!(parse_parts("a{}b").is_err());
    }

    #[test]
    fn unterminated_interpolation_error() {
        assert!(parse_parts("a{b").is_err());
    }

    #[test]
    fn simple_variable_interp() {
        let src = r#"
let name = "World";
"Hello, {name}!"
"#;
        assert_eq!(interp_eval(src), "Hello, World!");
    }

    #[test]
    fn arithmetic_in_interp() {
        let src = r#"
let x = 6;
let y = 7;
"The answer is {x * y}."
"#;
        assert_eq!(interp_eval(src), "The answer is 42.");
    }

    #[test]
    fn literal_prefix_and_suffix() {
        let parts = parse_parts("Hello, {name}!")
            .unwrap()
            .expect("should have parts");
        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0], StringPart::Literal(s) if s == "Hello, "));
        assert!(matches!(&parts[1], StringPart::Expr(_)));
        assert!(matches!(&parts[2], StringPart::Literal(s) if s == "!"));
    }
}
