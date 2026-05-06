//! RES-401: tuples — literals, element access, destructuring let.
//!
//! Owns:
//! - [`parse_paren_or_tuple`]: dispatch from the parser's `(` arm —
//!   distinguishes `()` (unit), `(expr)` (grouping), and `(a, b, ...)`
//!   (tuple).
//! - [`parse_let_tuple_destructure`]: dispatch from the parser's `let (`
//!   arm — produces a `Node::LetTupleDestructure`.
//! - [`eval_tuple_literal`]: interpreter helper.
//! - [`eval_tuple_index`]: interpreter helper for `t.N`.
//! - [`bind_tuple_destructure`]: interpreter helper for the
//!   destructuring let.
//!
//! Type checking is inline in `typechecker.rs` (treats every tuple
//! element as `Type::Any`); a follow-up ticket extends `Type` with a
//! dedicated tuple shape.
//!
//! The tuple AST is stored as three new `Node` variants and one new
//! `Value` variant in `main.rs` (per the feature-isolation pattern,
//! these are the only main.rs touch points).

use crate::span::Span;
use crate::{Interpreter, Node, Parser, RResult, Token, Value};

/// Parser entry — called from the `Token::LeftParen` prefix arm in
/// `main.rs`. On entry, `parser.current_token` is `(`. On exit,
/// `parser.current_token` sits on the closing `)`.
///
/// Disambiguation:
/// - `()` → `TupleLiteral { items: [] }` (the unit value).
/// - `(expr)` → grouped expression (no tuple wrapper).
/// - `(expr, ...)` → `TupleLiteral { items }`.
pub(crate) fn parse_paren_or_tuple(parser: &mut Parser) -> Option<Node> {
    let open_span = parser.span_at_current();
    parser.next_token(); // skip `(`

    // `()` — unit tuple.
    if parser.current_token == Token::RightParen {
        return Some(Node::TupleLiteral {
            items: Vec::new(),
            span: open_span,
        });
    }

    // First element. parse_expression leaves current_token on the
    // last token it consumed.
    let first = parser.parse_expression(0)?;
    parser.next_token();

    if parser.current_token == Token::RightParen {
        // Single expression in parens — keep grouping semantics.
        return Some(first);
    }
    if parser.current_token != Token::Comma {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "Expected `,` or `)` after expression in parens, found {}",
            tok
        ));
        return Some(first);
    }

    // Tuple — collect remaining items.
    let mut items = vec![first];
    while parser.current_token == Token::Comma {
        parser.next_token(); // skip `,`
        // Trailing comma allowed: `(a, b,)` — the `)` after a trailing
        // comma falls through to the close branch below.
        if parser.current_token == Token::RightParen {
            break;
        }
        let item = parser.parse_expression(0)?;
        items.push(item);
        parser.next_token();
    }

    if parser.current_token != Token::RightParen {
        let tok = parser.current_token.clone();
        parser.record_error(format!("Expected `)` closing tuple literal, found {}", tok));
    }

    Some(Node::TupleLiteral {
        items,
        span: open_span,
    })
}

/// Parser entry for `let (a, b, ...) = expr;`. Called from
/// `parse_let_statement` when, after the `let` keyword, the next token
/// is `(`. On entry, `parser.current_token` is `(`. On exit,
/// `parser.current_token` sits on the last token of the value expression
/// (the `;` is handled by the caller).
pub(crate) fn parse_let_tuple_destructure(parser: &mut Parser, stmt_span: Span) -> Node {
    parser.next_token(); // skip `(`

    let mut names: Vec<String> = Vec::new();
    let mut first = true;
    loop {
        if parser.current_token == Token::RightParen {
            break;
        }
        if !first {
            if parser.current_token != Token::Comma {
                let tok = parser.current_token.clone();
                parser.record_error(format!(
                    "Expected `,` or `)` in tuple destructure, found {}",
                    tok
                ));
                break;
            }
            parser.next_token(); // skip `,`
            // Trailing comma support: `(a, b,)`.
            if parser.current_token == Token::RightParen {
                break;
            }
        }
        match &parser.current_token {
            Token::Identifier(n) => names.push(n.clone()),
            other => {
                let tok = other.clone();
                parser.record_error(format!(
                    "Expected identifier in tuple destructure, found {}",
                    tok
                ));
            }
        }
        parser.next_token();
        first = false;
    }
    // Now sit on `)`.
    parser.next_token(); // skip `)`

    if parser.current_token != Token::Assign {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "Expected `=` after tuple destructure pattern, found {}",
            tok
        ));
        return Node::LetTupleDestructure {
            names,
            value: Box::new(Node::IntegerLiteral {
                value: 0,
                span: Span::default(),
            }),
            span: stmt_span,
        };
    }
    parser.next_token(); // skip `=`

    let value = parser.parse_expression(0).unwrap_or(Node::IntegerLiteral {
        value: 0,
        span: Span::default(),
    });

    if parser.peek_token == Token::Semicolon {
        parser.next_token();
    }

    Node::LetTupleDestructure {
        names,
        value: Box::new(value),
        span: stmt_span,
    }
}

/// Interpreter helper for `Node::TupleLiteral`. Each item is evaluated
/// left-to-right and collected into a `Value::Tuple`.
pub(crate) fn eval_tuple_literal(interp: &mut Interpreter, items: &[Node]) -> RResult<Value> {
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        out.push(interp.eval(it)?);
    }
    Ok(Value::Tuple(out))
}

/// Interpreter helper for `Node::TupleIndex`. Returns a clear runtime
/// diagnostic on out-of-range or non-tuple targets.
pub(crate) fn eval_tuple_index(
    interp: &mut Interpreter,
    tuple: &Node,
    index: usize,
    span: Span,
) -> RResult<Value> {
    let v = interp.eval(tuple)?;
    match v {
        Value::Tuple(items) => items.get(index).cloned().ok_or_else(|| {
            format!(
                "{}:{}: tuple index {} out of range (length {})",
                span.start.line,
                span.start.column,
                index,
                items.len()
            )
        }),
        // RES-928: tuple-struct constructors produce a `Value::Struct`
        // with synthesized field names "0", "1", ..., so `.N` against a
        // struct should look up the field with that name rather than
        // erroring as "non-tuple".
        Value::Struct { name, fields } => {
            let key = index.to_string();
            fields
                .into_iter()
                .find(|(n, _)| n == &key)
                .map(|(_, v)| v)
                .ok_or_else(|| {
                    format!(
                        "{}:{}: struct {} has no positional field `.{}`",
                        span.start.line, span.start.column, name, index
                    )
                })
        }
        other => Err(format!(
            "{}:{}: cannot index `.{}` on non-tuple value `{}`",
            span.start.line, span.start.column, index, other
        )),
    }
}

/// Interpreter helper for `Node::LetTupleDestructure`. Evaluates the
/// RHS, asserts it's a tuple of the right arity, and binds each name
/// in the current environment.
pub(crate) fn bind_tuple_destructure(
    interp: &mut Interpreter,
    names: &[String],
    value: &Node,
    span: Span,
) -> RResult<Value> {
    let v = interp.eval(value)?;
    let items = match v {
        Value::Tuple(items) => items,
        other => {
            return Err(format!(
                "{}:{}: cannot destructure non-tuple value `{}` into ({} names)",
                span.start.line,
                span.start.column,
                other,
                names.len()
            ));
        }
    };
    if items.len() != names.len() {
        return Err(format!(
            "{}:{}: tuple destructure expects {} elements, got {}",
            span.start.line,
            span.start.column,
            names.len(),
            items.len()
        ));
    }
    for (n, val) in names.iter().zip(items) {
        interp.env.set(n.clone(), val);
    }
    Ok(Value::Void)
}

#[cfg(test)]
mod tests {
    use crate::{Interpreter, Lexer, Parser, Value};

    /// Parse + run `src`. Returns the program's final result for
    /// expression-style smoke tests, or `Value::Void` for procedural
    /// programs that print via `println`. Tests assert against
    /// captured side effects via the runtime's known semantics.
    fn run(src: &str) -> Value {
        let lexer = Lexer::new(src.to_string());
        let mut parser = Parser::new(lexer);
        let program = parser.parse_program();
        assert!(
            parser.errors.is_empty(),
            "parse errors: {:?}",
            parser.errors
        );
        let mut interp = Interpreter::new();
        interp.eval(&program).expect("eval failed")
    }

    #[test]
    fn empty_parens_is_unit_tuple() {
        let v = run("()");
        assert!(matches!(&v, Value::Tuple(items) if items.is_empty()));
    }

    #[test]
    fn paren_around_single_expr_is_grouped() {
        // `(42)` parses as the integer literal `42`, NOT a 1-tuple.
        let v = run("(42)");
        assert!(matches!(&v, Value::Int(42)));
    }

    #[test]
    fn pair_tuple_evaluates() {
        let v = run("(10, 20)");
        match v {
            Value::Tuple(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(items[0], Value::Int(10)));
                assert!(matches!(items[1], Value::Int(20)));
            }
            other => panic!("expected Tuple, got {:?}", other),
        }
    }

    #[test]
    fn triple_tuple_with_mixed_types() {
        let v = run("(1, \"two\", true)");
        match v {
            Value::Tuple(items) => {
                assert_eq!(items.len(), 3);
                assert!(matches!(items[0], Value::Int(1)));
                assert!(matches!(&items[1], Value::String(s) if s == "two"));
                assert!(matches!(items[2], Value::Bool(true)));
            }
            other => panic!("expected Tuple, got {:?}", other),
        }
    }

    #[test]
    fn element_access_returns_correct_item() {
        let v = run("(10, 20, 30).1");
        assert!(matches!(v, Value::Int(20)));
    }

    #[test]
    fn element_access_on_named_binding() {
        let v = run("let p = (5, 7); p.0 + p.1");
        assert!(matches!(v, Value::Int(12)));
    }

    #[test]
    fn element_access_out_of_range_errors() {
        let lexer = Lexer::new("(1, 2).5".to_string());
        let mut parser = Parser::new(lexer);
        let program = parser.parse_program();
        assert!(parser.errors.is_empty());
        let mut interp = Interpreter::new();
        let err = interp.eval(&program).unwrap_err();
        assert!(
            err.contains("out of range"),
            "expected `out of range`, got: {}",
            err
        );
    }

    #[test]
    fn destructure_let_binds_all_names() {
        let v = run("let (a, b) = (10, 20); a + b");
        assert!(matches!(v, Value::Int(30)));
    }

    #[test]
    fn destructure_arity_mismatch_errors() {
        let lexer = Lexer::new("let (a, b, c) = (1, 2);".to_string());
        let mut parser = Parser::new(lexer);
        let program = parser.parse_program();
        assert!(parser.errors.is_empty());
        let mut interp = Interpreter::new();
        let err = interp.eval(&program).unwrap_err();
        assert!(
            err.contains("expects 3 elements"),
            "expected arity error, got: {}",
            err
        );
    }

    #[test]
    fn destructure_non_tuple_errors() {
        let lexer = Lexer::new("let (a, b) = 42;".to_string());
        let mut parser = Parser::new(lexer);
        let program = parser.parse_program();
        assert!(parser.errors.is_empty());
        let mut interp = Interpreter::new();
        let err = interp.eval(&program).unwrap_err();
        assert!(
            err.contains("non-tuple"),
            "expected non-tuple error, got: {}",
            err
        );
    }

    #[test]
    fn nested_tuple() {
        let v = run("((1, 2), (3, 4))");
        match &v {
            Value::Tuple(outer) => {
                assert_eq!(outer.len(), 2);
                match (&outer[0], &outer[1]) {
                    (Value::Tuple(a), Value::Tuple(b)) => {
                        assert_eq!(a.len(), 2);
                        assert_eq!(b.len(), 2);
                    }
                    _ => panic!("inner items must be tuples"),
                }
            }
            other => panic!("expected outer Tuple, got {:?}", other),
        }
    }

    #[test]
    fn tuple_display_format() {
        let v = run("(1, 2, 3)");
        assert_eq!(format!("{}", v), "(1, 2, 3)");
    }

    #[test]
    fn unit_tuple_display() {
        let v = run("()");
        assert_eq!(format!("{}", v), "()");
    }

    #[test]
    fn tuple_index_on_non_tuple_errors() {
        let lexer = Lexer::new("let x = 42; x.0".to_string());
        let mut parser = Parser::new(lexer);
        let program = parser.parse_program();
        assert!(parser.errors.is_empty());
        let mut interp = Interpreter::new();
        let err = interp.eval(&program).unwrap_err();
        assert!(
            err.contains("non-tuple"),
            "expected non-tuple error, got: {}",
            err
        );
    }
}
