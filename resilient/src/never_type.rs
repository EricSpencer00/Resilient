//! RES-2578: `!` (never) type for functions that don't return.
//!
//! The never type is the bottom type: a function declared `-> !` promises
//! to **never return** to its caller — it panics, exits, loops forever,
//! or calls another `-> !` function.
//!
//! ## Syntax
//! ```text
//! fn abort(string msg) -> ! {
//!     println("FATAL: " + msg);
//!     exit(1);
//! }
//! ```
//!
//! ## Typechecker pass
//!
//! `check` validates every `fn ... -> !` declaration:
//!
//! 1. The function body must not contain a reachable `return <value>` at
//!    the top of the body block. A bare `return;` is also invalid (it
//!    returns control, just with no value). The function is expected to
//!    exit via an infinite loop, a call to another `-> !` function,
//!    or a runtime-exit builtin (`exit`, `panic`, `abort`).
//! 2. No other constraint is enforced today — proving divergence in
//!    general is undecidable, so we accept any body that doesn't
//!    contain an obvious returning path.
//!
//! Errors are surfaced as compiler diagnostics before the interpreter runs.

use crate::Node;
use crate::span::Span;

/// Typecheck pass for `fn ... -> !` declarations.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    let mut errors: Vec<String> = Vec::new();
    for s in stmts {
        check_node(&s.node, source_path, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn check_node(node: &Node, source_path: &str, errors: &mut Vec<String>) {
    match node {
        Node::Function {
            name,
            return_type,
            body,
            span,
            ..
        } if return_type.as_deref() == Some("!") => {
            check_never_fn_body(name, body, *span, source_path, errors);
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                check_node(m, source_path, errors);
            }
        }
        Node::ModuleDecl { body, .. } => {
            for child in body {
                check_node(child, source_path, errors);
            }
        }
        _ => {}
    }
}

/// For a `-> !` function, check that no statement-bearing control-flow path
/// contains a reachable `return`. Nested function declarations and literals
/// are separate return scopes and are deliberately not descended into.
fn check_never_fn_body(
    fn_name: &str,
    body: &Node,
    fn_span: Span,
    source_path: &str,
    errors: &mut Vec<String>,
) {
    let stmts = match body {
        Node::Block { stmts, .. } => stmts,
        _ => return,
    };

    for stmt in stmts {
        reject_nested_returns(stmt, fn_name, source_path, errors);
    }

    // If the body is empty or contains no obvious divergence call, warn.
    // (Advisory only — not an error, since we can't prove divergence.)
    let _ = fn_span; // reserved for future divergence heuristic
}

/// Visit statement-bearing control-flow nodes without crossing a nested
/// function boundary. A return inside a nested function is a return from that
/// function, not from the enclosing `-> !` function.
fn reject_nested_returns(node: &Node, fn_name: &str, source_path: &str, errors: &mut Vec<String>) {
    match node {
        Node::ReturnStatement { value, span } => {
            let loc = fmt_loc(source_path, *span);
            if value.is_some() {
                errors.push(format!(
                    "{loc}: function `{fn_name}` is declared `-> !` (never returns) \
                     but contains a `return` statement"
                ));
            } else {
                errors.push(format!(
                    "{loc}: function `{fn_name}` is declared `-> !` (never returns) \
                     but contains a bare `return;`"
                ));
            }
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                reject_nested_returns(stmt, fn_name, source_path, errors);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            reject_nested_returns(consequence, fn_name, source_path, errors);
            if let Some(alt) = alternative {
                reject_nested_returns(alt, fn_name, source_path, errors);
            }
        }
        Node::WhileStatement { body, .. }
        | Node::ForInStatement { body, .. }
        | Node::LiveBlock { body, .. }
        | Node::UnsafeBlock { body, .. }
        | Node::BenchBlock { body, .. } => {
            reject_nested_returns(body, fn_name, source_path, errors);
        }
        Node::Match { arms, .. } => {
            for (_, _, body) in arms {
                reject_nested_returns(body, fn_name, source_path, errors);
            }
        }
        Node::TryCatch { body, handlers, .. } => {
            for stmt in body {
                reject_nested_returns(stmt, fn_name, source_path, errors);
            }
            for (_, handler_body) in handlers {
                for stmt in handler_body {
                    reject_nested_returns(stmt, fn_name, source_path, errors);
                }
            }
        }
        // Nested functions have independent return contracts.
        Node::Function { .. } | Node::FunctionLiteral { .. } => {}
        _ => {}
    }
}

fn fmt_loc(source_path: &str, span: Span) -> String {
    if span.start.line == 0 {
        source_path.to_string()
    } else {
        format!("{}:{}:{}", source_path, span.start.line, span.start.column)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::{Lexer, Parser};

    fn parse_src(src: &str) -> crate::Node {
        let lexer = Lexer::new(src);
        let mut parser = Parser::new(lexer);
        parser.parse_program()
    }

    #[test]
    fn never_return_type_parses() {
        // `-> !` should parse without errors.
        let src = "fn abort(string msg) -> ! { exit(1); } abort(\"fail\");";
        let prog = parse_src(src);
        let result = super::check(&prog, "test.rz");
        assert!(result.is_ok(), "expected no errors, got: {:?}", result);
    }

    #[test]
    fn never_fn_with_return_value_errors() {
        let src = "fn bad() -> ! { return 42; }";
        let prog = parse_src(src);
        let result = super::check(&prog, "test.rz");
        assert!(
            result.is_err(),
            "expected error for return in -> ! function"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("never returns"),
            "error should mention never returns, got: {err}"
        );
    }

    #[test]
    fn never_fn_with_bare_return_errors() {
        let src = "fn bad() -> ! { return; }";
        let prog = parse_src(src);
        let result = super::check(&prog, "test.rz");
        assert!(
            result.is_err(),
            "expected error for bare return in -> ! function"
        );
    }

    #[test]
    fn never_fn_infinite_loop_ok() {
        // An infinite loop body is valid for a -> ! function.
        let src = "fn spin() -> ! { loop { } }";
        let prog = parse_src(src);
        let result = super::check(&prog, "test.rz");
        assert!(
            result.is_ok(),
            "infinite loop is valid -> !, got: {:?}",
            result
        );
    }

    #[test]
    fn regular_fn_unaffected() {
        // Normal functions are not affected by the never-type check.
        let src = "fn foo() -> int { return 42; }";
        let prog = parse_src(src);
        let result = super::check(&prog, "test.rz");
        assert!(
            result.is_ok(),
            "regular fn should not be checked, got: {:?}",
            result
        );
    }

    #[test]
    fn nested_if_return_in_never_fn_errors() {
        let src = "fn bad() -> ! { if true { return 42; } exit(1); }";
        let prog = parse_src(src);
        let result = super::check(&prog, "test.rz");
        assert!(result.is_err(), "nested return must violate -> !");
    }

    #[test]
    fn nested_match_and_try_returns_in_never_fn_error() {
        let src = "fn bad(int x) -> ! { \
            match x { 0 => { return; }, _ => { exit(1); } } \
            try { exit(1); } catch Timeout { return; } \
        }";
        let prog = parse_src(src);
        let result = super::check(&prog, "test.rz");
        assert!(result.is_err(), "structured returns must violate -> !");
        let err = result.unwrap_err();
        assert!(
            err.matches("never returns").count() >= 2,
            "missing nested diagnostics: {err}"
        );
    }

    #[test]
    fn nested_function_return_does_not_invalidate_outer_never_fn() {
        let src = "fn outer() -> ! { fn inner() -> int { return 1; } exit(1); }";
        let prog = parse_src(src);
        let result = super::check(&prog, "test.rz");
        assert!(
            result.is_ok(),
            "nested function has its own return scope: {result:?}"
        );
    }
}
