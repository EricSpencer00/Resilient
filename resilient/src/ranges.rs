//! RES-291: integer range expressions `lo..hi` (half-open) and `lo..=hi`
//! (inclusive) for iteration.
//!
//! Initial scope is intentionally narrow — ranges are only legal in two
//! USAGE contexts:
//!
//! 1. Right side of `for x in <range> { ... }`.
//! 2. Right side of a `let r = <range>;` binding.
//!
//! The parser hook lives at the bottom of `parse_for_in_statement` and
//! `parse_let_statement`, so a stray range elsewhere falls through to a
//! normal parser error. Iteration is lazy: the tree-walker materializes
//! one `Value::Int` at a time, never an array.
//!
//! VM and JIT support are explicit follow-ups — for now, both paths
//! reject ranges with an "unsupported" message rather than silently
//! producing wrong results.
//!
//! Layout of this file:
//!
//! - `parse_range_tail` — call-site helper used by `for-in` and `let`.
//! - `check` — typechecker pass: `lo` and `hi` must be `Int`.
//! - `iterate_range` — driver loop used by `for-in` evaluation.
//!
//! Token reuse: the existing `Token::DotDot` (added in RES-330 for
//! quantifier ranges) doubles as the half-open separator. Inclusive
//! `..=` is detected by peeking for `Token::Assign` immediately after a
//! `DotDot` rather than introducing a new lexer token.

use crate::{Node, Token, span};

/// Try to extend an already-parsed lower-bound expression into a full
/// `Range` node. Caller passes the parsed `lo` expression; this function
/// inspects the parser's current token. If it is `..` (or `..=`), the
/// range is built and returned as `Some(node)`. Otherwise the function
/// returns `None` and the caller keeps `lo` as the original expression.
///
/// On success, the parser's `current_token` sits on the last token of
/// `hi` (the standard RES-014 invariant — caller calls `next_token()`
/// once to advance past it).
pub(crate) fn parse_range_tail(
    parser: &mut crate::Parser,
    lo: Node,
    span: span::Span,
) -> Option<Node> {
    if parser.current_token != Token::DotDot {
        return None;
    }
    parser.next_token(); // skip `..`
    // Inclusive form `..=` — the lexer emits `..` followed by `=`, so
    // we detect it here without a dedicated lexer token.
    let inclusive = parser.current_token == Token::Assign;
    if inclusive {
        parser.next_token(); // skip `=`
    }
    let hi = parser
        .parse_expression(0)
        .unwrap_or(Node::IntegerLiteral { value: 0, span });
    Some(Node::Range {
        lo: Box::new(lo),
        hi: Box::new(hi),
        inclusive,
        span,
    })
}

/// Typecheck pass — walk the program and reject any `Range` whose
/// bounds are not `Int`, or whose surrounding context disallows ranges.
/// Today the only legal contexts are `ForInStatement.iterable` and the
/// RHS of a `LetStatement.value`.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };
    for stmt in stmts {
        check_stmt(&stmt.node, source_path)?;
    }
    Ok(())
}

fn check_stmt(node: &Node, source_path: &str) -> Result<(), String> {
    match node {
        Node::ForInStatement { iterable, body, .. } => {
            // Ranges are allowed at the top of `iterable` — descend
            // through it but allow the immediate Range. Anywhere else
            // (including inside the body) ranges are still rejected.
            check_iterable_or_let_rhs(iterable, source_path)?;
            check_stmt(body, source_path)?;
            Ok(())
        }
        Node::LetStatement { value, .. } => {
            check_iterable_or_let_rhs(value, source_path)?;
            Ok(())
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                check_stmt(s, source_path)?;
            }
            Ok(())
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            check_stmt(consequence, source_path)?;
            if let Some(alt) = alternative {
                check_stmt(alt, source_path)?;
            }
            Ok(())
        }
        Node::WhileStatement { body, .. } => check_stmt(body, source_path),
        Node::Function { body, .. } => check_stmt(body, source_path),
        Node::Range { span, .. } => Err(format_err(
            span,
            source_path,
            "range expressions are only allowed in `for x in <range>` or `let r = <range>;`",
        )),
        _ => Ok(()),
    }
}

fn check_iterable_or_let_rhs(node: &Node, source_path: &str) -> Result<(), String> {
    match node {
        Node::Range { lo, hi, span, .. } => {
            if !is_intlike(lo) {
                return Err(format_err(
                    span,
                    source_path,
                    "range lower bound must be an integer expression",
                ));
            }
            if !is_intlike(hi) {
                return Err(format_err(
                    span,
                    source_path,
                    "range upper bound must be an integer expression",
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Conservative shape check: accept literals, identifiers, calls,
/// arithmetic infix exprs. Anything else (string literal, array
/// literal, etc.) is rejected. Final type confirmation happens at
/// runtime — this is a lightweight gate to catch obvious misuse early.
fn is_intlike(n: &Node) -> bool {
    matches!(
        n,
        Node::IntegerLiteral { .. }
            | Node::Identifier { .. }
            | Node::PrefixExpression { .. }
            | Node::InfixExpression { .. }
            | Node::CallExpression { .. }
            | Node::IndexExpression { .. }
            | Node::FieldAccess { .. }
    )
}

fn format_err(span: &span::Span, source_path: &str, msg: &str) -> String {
    if source_path.is_empty() {
        format!("{}:{}: {}", span.start.line, span.start.column, msg)
    } else {
        format!(
            "{}:{}:{}: {}",
            source_path, span.start.line, span.start.column, msg
        )
    }
}

/// Iterator over the integer values produced by a range. `inclusive`
/// includes `hi`; `!inclusive` is the standard half-open `[lo, hi)`.
/// Empty when `lo > hi`.
pub(crate) fn iterate_range(lo: i64, hi: i64, inclusive: bool) -> impl Iterator<Item = i64> {
    let end = if inclusive { hi.saturating_add(1) } else { hi };
    // `lo..end` is empty iff lo >= end, which is the right behaviour
    // for both half-open (`lo > hi`) and inclusive (`lo > hi`) cases.
    lo..end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_program(src: &str) -> Node {
        let lexer = crate::Lexer::new(src.to_string());
        let mut p = crate::Parser::new(lexer);
        p.parse_program()
    }

    #[test]
    fn half_open_range_iterates_lo_inclusive_hi_exclusive() {
        let v: Vec<i64> = iterate_range(0, 5, false).collect();
        assert_eq!(v, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn inclusive_range_includes_hi() {
        let v: Vec<i64> = iterate_range(1, 5, true).collect();
        assert_eq!(v, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn empty_range_when_lo_eq_hi_half_open() {
        let v: Vec<i64> = iterate_range(3, 3, false).collect();
        assert!(v.is_empty());
    }

    #[test]
    fn singleton_range_when_lo_eq_hi_inclusive() {
        let v: Vec<i64> = iterate_range(7, 7, true).collect();
        assert_eq!(v, vec![7]);
    }

    #[test]
    fn empty_range_when_lo_gt_hi() {
        let v: Vec<i64> = iterate_range(10, 3, false).collect();
        assert!(v.is_empty());
        let v: Vec<i64> = iterate_range(10, 3, true).collect();
        assert!(v.is_empty());
    }

    #[test]
    fn parse_for_in_range_half_open() {
        let prog = parse_program("for i in 0..3 { println(i); }");
        let stmts = match prog {
            crate::Node::Program(stmts) => stmts,
            _ => panic!("expected program"),
        };
        match &stmts[0].node {
            crate::Node::ForInStatement { iterable, .. } => match iterable.as_ref() {
                crate::Node::Range { inclusive, .. } => assert!(!inclusive),
                other => panic!("expected Range, got {:?}", other),
            },
            other => panic!("expected ForInStatement, got {:?}", other),
        }
    }

    #[test]
    fn parse_for_in_range_inclusive() {
        let prog = parse_program("for i in 0..=3 { println(i); }");
        let stmts = match prog {
            crate::Node::Program(stmts) => stmts,
            _ => panic!("expected program"),
        };
        match &stmts[0].node {
            crate::Node::ForInStatement { iterable, .. } => match iterable.as_ref() {
                crate::Node::Range { inclusive, .. } => assert!(inclusive),
                other => panic!("expected Range, got {:?}", other),
            },
            other => panic!("expected ForInStatement, got {:?}", other),
        }
    }

    #[test]
    fn parse_let_range_binds_a_range_node() {
        let prog = parse_program("let r = 0..5;\n");
        let stmts = match prog {
            crate::Node::Program(stmts) => stmts,
            _ => panic!("expected program"),
        };
        match &stmts[0].node {
            crate::Node::LetStatement { value, .. } => match value.as_ref() {
                crate::Node::Range { inclusive, .. } => assert!(!inclusive),
                other => panic!("expected Range on let RHS, got {:?}", other),
            },
            other => panic!("expected LetStatement, got {:?}", other),
        }
    }
}
