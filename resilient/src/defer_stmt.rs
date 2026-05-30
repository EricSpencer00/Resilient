//! RES-2579: `defer` statement — execute an expression when the enclosing
//! function returns, in last-in-first-out order (Go-style defer).
//!
//! ## Syntax
//! ```text
//! fn with_cleanup() {
//!     defer println("cleaned up");
//!     // ... body ...
//! }
//! ```
//!
//! ## Semantics
//!
//! Each `defer <expr>;` encountered during function execution pushes `<expr>`
//! onto the function's defer stack. When the function exits — whether by
//! reaching the end of its body, executing `return`, or propagating a runtime
//! error — the deferred expressions are evaluated in LIFO order (last deferred
//! = first executed). Deferred expressions are evaluated for their side
//! effects; their return values are discarded.
//!
//! Defer captures the **current environment** at the point of the `defer`
//! statement (not the environment at exit time) — variable bindings are
//! snapshotted when the defer is registered.
//!
//! ## Limitations (MVP)
//!
//! - Only function-scope defer is supported. Defers inside loops accumulate
//!   on the function stack; they do not fire at loop exit.
//! - Errors thrown inside a deferred expression are currently surfaced to the
//!   caller rather than being suppressed.

use crate::Node;

/// Typecheck pass: validates `defer` statements appear only inside function
/// bodies (not at top-level). A `defer` at the top level of a program would
/// never execute because there is no surrounding function return event.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    let mut errors: Vec<String> = Vec::new();
    for s in stmts {
        check_top_level_defer(&s.node, source_path, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// Top-level statements must not be bare `defer` (outside any function body).
fn check_top_level_defer(node: &Node, source_path: &str, errors: &mut Vec<String>) {
    match node {
        Node::DeferStatement { span, .. } => {
            let loc = fmt_loc(source_path, *span);
            errors.push(format!(
                "{loc}: `defer` cannot appear at the top level — \
                 it must be inside a function body"
            ));
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                check_top_level_defer(m, source_path, errors);
            }
        }
        Node::ModuleDecl { body, .. } => {
            for child in body {
                check_top_level_defer(child, source_path, errors);
            }
        }
        _ => {}
    }
}

fn fmt_loc(source_path: &str, span: crate::span::Span) -> String {
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

    fn parse(src: &str) -> crate::Node {
        let lexer = Lexer::new(src);
        let mut parser = Parser::new(lexer);
        parser.parse_program()
    }

    fn run(src: &str) -> crate::RunResult {
        crate::run_program(src)
    }

    #[test]
    fn defer_parses_ok() {
        let prog = parse("fn f() { defer println(\"hi\"); } f();");
        assert!(super::check(&prog, "test.rz").is_ok());
    }

    #[test]
    fn top_level_defer_errors() {
        let prog = parse("defer println(\"hi\");");
        let result = super::check(&prog, "test.rz");
        assert!(result.is_err(), "expected error for top-level defer");
        assert!(result.unwrap_err().contains("top level"));
    }

    #[test]
    fn defer_executes_after_function_body() {
        // The deferred println runs AFTER the body println.
        let r = run(r#"
fn greet() {
    defer println("goodbye");
    println("hello");
}
greet();
"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.lines().collect();
        // "hello" before "goodbye"
        assert!(
            lines.iter().position(|&l| l == "hello") < lines.iter().position(|&l| l == "goodbye"),
            "defer should run after body: got {:?}",
            r.stdout
        );
    }

    #[test]
    fn defer_lifo_order() {
        // Multiple defers execute in LIFO order.
        let r = run(r#"
fn f() {
    defer println("first");
    defer println("second");
    defer println("third");
}
f();
"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
        let relevant: Vec<&str> = lines
            .iter()
            .filter(|&&l| l == "first" || l == "second" || l == "third")
            .copied()
            .collect();
        assert_eq!(
            relevant,
            vec!["third", "second", "first"],
            "defers should fire LIFO, got: {:?}",
            r.stdout
        );
    }

    #[test]
    fn defer_with_early_return() {
        // Deferred call runs even when function returns early.
        let r = run(r#"
fn f(int x) -> string {
    defer println("deferred");
    if x > 0 {
        return "positive";
    }
    "non-positive"
}
let r = f(5);
println(r);
"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(
            r.stdout.contains("deferred"),
            "defer should run on early return, got: {:?}",
            r.stdout
        );
        assert!(
            r.stdout.contains("positive"),
            "return value should be 'positive', got: {:?}",
            r.stdout
        );
    }
}
