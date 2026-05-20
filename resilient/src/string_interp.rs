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

// RES-1605: `check` is no longer called from `EXTENSION_PASSES`
// (the body is `Ok(())`). The module-level `dead_code` allow keeps
// the fn around for symmetry with the other extension-point passes;
// re-adding the typechecker call when the pass becomes meaningful
// is a one-line append in `typechecker.rs`.
#![allow(dead_code)]

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

    // RES-1792: pre-size to (placeholder-count * 2 + 1) — one Literal
    // per `{...}` placeholder plus a trailing Literal. Matches the
    // typical 1-3-placeholder shape and is computed in O(N) via
    // `matches`. Called for every interpolated string literal in source.
    let mut parts: Vec<StringPart> = Vec::with_capacity(raw.matches('{').count() * 2 + 1);
    // RES-1832: pre-size to 16 — most literal segments between
    // interpolation placeholders fit in 16 bytes without realloc.
    let mut literal_buf = String::with_capacity(16);
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
                //
                // RES-1479: `mem::take` swaps in the default (empty
                // String) and returns the original — exactly the
                // clone-then-clear shape we need, but without the
                // String alloc. The previous `literal_buf.clone() +
                // literal_buf.clear()` allocated a new String for
                // every interpolation `{expr}` boundary in a parsed
                // template, dropping the literal_buf's owned heap
                // bytes via `clear`. Now the literal_buf's owned
                // bytes move into the StringPart directly.
                if !literal_buf.is_empty() {
                    parts.push(StringPart::Literal(std::mem::take(&mut literal_buf)));
                }

                // Collect source up to the matching `}`.
                // RES-1822: pre-size to 16 — typical interpolation
                // expressions are 5-30 chars (`name`, `arr[i]`,
                // `x + 1`). Saves the 0→8→16→… doubling chain on
                // every placeholder collected.
                let mut expr_src = String::with_capacity(16);
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
                let lexer = Lexer::new(&expr_src);
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
    // RES-1832: pre-size to 32 — covers most short interpolated
    // strings without realloc; grows automatically for longer output.
    let mut out = String::with_capacity(32);
    for part in parts {
        match part {
            StringPart::Literal(s) => out.push_str(s),
            StringPart::Expr(expr) => {
                let val = interp.eval(expr)?;
                // RES-2254: append the value directly to `out` instead
                // of materializing it into a `String` first. For
                // numeric / bool / "other" arms, `value_to_string`
                // previously allocated a fresh `String` (via
                // `i.to_string()`, `format!()`) just to immediately
                // copy it into `out` via `push_str`. `write!(out,
                // "{}", x)` formats straight into `out`'s buffer.
                append_value(&mut out, val);
            }
        }
    }
    Ok(Value::String(out))
}

/// Append a [`Value`]'s plain text form directly to an output buffer.
///
/// Strings yield their raw contents (no surrounding quotes).
/// Integers, floats, and booleans format via `Display`.
/// All other values (arrays, structs, maps, etc.) use the `Display`
/// implementation of `Value`, which matches what `println` would show.
///
/// RES-2254: writes through `std::fmt::Write` instead of returning a
/// fresh `String`. For interpolation-heavy code (log messages, debug
/// prints), this eliminates one `String` allocation per interpolated
/// expression of a non-String value.
fn append_value(out: &mut String, v: Value) {
    use std::fmt::Write;
    match v {
        Value::String(s) => out.push_str(&s),
        Value::Int(i) => {
            let _ = write!(out, "{}", i);
        }
        Value::Float(f) => {
            let _ = write!(out, "{}", f);
        }
        Value::Bool(b) => {
            let _ = write!(out, "{}", b);
        }
        other => {
            let _ = write!(out, "{}", other);
        }
    }
}

// ---------- Type-check pass ----------

/// RES-221: type-check interpolated string sub-expressions.
///
/// Walks all `Node::InterpolatedString` nodes in the program and
/// runs the typechecker on each `StringPart::Expr`. Any expression
/// that fails to type-check (e.g. a reference to an undefined variable)
/// is surfaced as an error. Primitive types (Int, Float, Bool, String)
/// are accepted unconditionally; complex types (Array, Struct, etc.) are
/// accepted too — they stringify via their `Display` impl at runtime.
///
/// Parse-time validation (unterminated braces, empty interpolations,
/// syntax errors in sub-expressions) is already handled by [`parse_parts`].
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    let mut tc = crate::typechecker::TypeChecker::new();
    // Pre-populate the typechecker with all top-level function and
    // struct definitions so interpolated expressions inside fn bodies
    // can reference other functions without false "undefined" errors.
    for stmt in stmts {
        if matches!(
            &stmt.node,
            Node::Function { .. }
                | Node::StructDecl { .. }
                | Node::ImplBlock { .. }
                | Node::Extern { .. }
        ) {
            let _ = tc.check_node(&stmt.node);
        }
    }
    // Now walk every statement and check InterpolatedString sub-exprs.
    for stmt in stmts {
        check_node_interp(&stmt.node, &mut tc, source_path)?;
    }
    Ok(())
}

fn check_node_interp(
    node: &Node,
    tc: &mut crate::typechecker::TypeChecker,
    source_path: &str,
) -> Result<(), String> {
    if let Node::InterpolatedString { parts, span } = node {
        for part in parts {
            if let StringPart::Expr(expr) = part {
                tc.check_node(expr).map_err(|e| {
                    if span.start.line > 0 {
                        format!(
                            "{}:{}:{}: in interpolated string: {}",
                            source_path, span.start.line, span.start.column, e
                        )
                    } else {
                        format!("in interpolated string: {e}")
                    }
                })?;
            }
        }
    }
    crate::uniqueness_walk::visit(node, &mut |n| {
        if let Node::InterpolatedString { parts, span } = n {
            for part in parts {
                if let StringPart::Expr(expr) = part
                    && let Err(e) = tc.check_node(expr)
                {
                    // Errors in nested nodes: we can't propagate from the
                    // closure, so emit as a warning-style diagnostic to stderr.
                    let loc = if span.start.line > 0 {
                        format!("{}:{}:{}", source_path, span.start.line, span.start.column)
                    } else {
                        source_path.to_string()
                    };
                    eprintln!("warning: {loc}: in interpolated string: {e}");
                }
            }
        }
    });
    Ok(())
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn interp_eval(src: &str) -> String {
        let lexer = crate::Lexer::new(src);
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
