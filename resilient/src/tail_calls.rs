//! RES-2592: tail call optimization (TCO) for self-recursive functions.
//!
//! Resilient targets embedded systems where stack space is scarce. Without TCO,
//! every recursive call consumes a stack frame; on bounded stacks this quickly
//! exhausts available memory. This module provides:
//!
//! 1. **Tail-position detection** — `is_tail_call(node, fn_name)` returns
//!    `true` when a `CallExpression` that calls `fn_name` appears in a
//!    syntactic tail position inside a function body.
//!
//! 2. **`#[must_tail_call]` attribute check** — `check(program, source_path)`
//!    walks every function annotated with `#[must_tail_call]` and emits a
//!    compile error if any self-recursive call in that function is NOT in
//!    tail position. This gives the author a static guarantee that TCO will
//!    fire at runtime.
//!
//! ## Tail positions
//!
//! A call `f(...)` is in tail position inside a function body iff it is the
//! value of the last expression that will be evaluated before the function
//! returns:
//!
//! - The final expression of a `Block { stmts }`.
//! - The value of a `ReturnStatement`.
//! - The consequence or alternative of an `IfStatement` that is itself in
//!   tail position.
//! - The arm body of a `Match` expression that is itself in tail position.
//! - **Not** the condition of an `if` or `while`.
//! - **Not** any argument to another call.
//! - **Not** the left/right of a binary expression.
//!
//! ## Feature isolation
//!
//! All TCO logic lives here. The core files touch only:
//!
//! - `feature_attrs.rs`: `"must_tail_call"` added to `is_known_attribute`.
//! - `typechecker.rs` `<EXTENSION_PASSES>`: one `crate::tail_calls::check(...)` call.
//! - `lib.rs` `apply_function`: the trampoline loop (see inline comment RES-2592).
//!
//! ## Runtime TCO mechanism
//!
//! `apply_function` in `lib.rs` checks `is_must_tail_call(name)` for each
//! `Value::Function` call. When true it:
//!
//! 1. Sets `tco_fn_name = Some(name.clone())` on the child interpreter.
//! 2. Evaluates the body in a `loop`.
//! 3. If the body result is `Value::TailCall(new_args)`, rebinds the
//!    function's parameters to `new_args` in the environment and continues
//!    the loop — no new stack frame.
//! 4. Any other result (including `Value::Return(v)`) breaks out of the
//!    loop and returns normally.
//!
//! The child interpreter emits `Value::TailCall(args)` when it encounters a
//! `CallExpression` whose callee matches `tco_fn_name`. Because
//! `#[must_tail_call]` is statically verified to allow only tail self-calls,
//! this signal is always consumed by the trampoline before propagating.
//!
//! ## Feature isolation
//!
//! All TCO logic lives here. The core files touch only:
//!
//! - `feature_attrs.rs`: `"must_tail_call"` added to `is_known_attribute`.
//! - `typechecker.rs` `<EXTENSION_PASSES>`: one `crate::tail_calls::check(...)` call.
//! - `lib.rs`:
//!   - `mod tail_calls;` declaration.
//!   - `Value::TailCall(Vec<Value>)` variant added to the `Value` enum.
//!   - `tco_fn_name: Option<String>` field on the `Interpreter` struct.
//!   - Trampoline loop in `apply_function` (see RES-2592 comment there).
//!   - Tail-call emit in `Node::CallExpression` eval (see RES-2592 comment).
//!
//! Mutual recursion TCO is out of scope for this PR (tracked as a follow-up).

use crate::Node;

// ---------------------------------------------------------------------------
// Runtime query — used by apply_function in lib.rs
// ---------------------------------------------------------------------------

/// Returns `true` when `fn_name` is annotated with `#[must_tail_call]`.
///
/// Called from `apply_function` to determine whether the trampoline loop
/// should be activated for this call frame. `find_kind` already does an
/// atomic fast-reject when no attributes have been recorded, so the common
/// case (no `#[must_tail_call]` annotations in the program) pays only an
/// atomic load.
pub fn is_must_tail_call(fn_name: &str) -> bool {
    let attrs = crate::feature_attrs::find_kind("must_tail_call");
    attrs.iter().any(|(name, _)| name == fn_name)
}

// ---------------------------------------------------------------------------
// Tail-position detection (public API)
// ---------------------------------------------------------------------------

/// Returns `true` when `node` is a `CallExpression` calling `fn_name` that
/// appears in a syntactic tail position.
///
/// The caller should pass the *body* `Node` of a function (typically a
/// `Block`) together with the function's own name.
///
/// This is a purely structural check — it does not evaluate or type-check.
#[allow(dead_code)]
pub fn is_tail_call(node: &Node, fn_name: &str) -> bool {
    is_tail_expr(node, fn_name)
}

/// Recursively check whether `node` is, or reachable-as, a tail call to
/// `fn_name`.
#[allow(dead_code)]
fn is_tail_expr(node: &Node, fn_name: &str) -> bool {
    match node {
        // A direct call to fn_name in tail position.
        Node::CallExpression { function, .. } => {
            matches!(function.as_ref(), Node::Identifier { name, .. } if name == fn_name)
        }

        // A block: only the last statement is in tail position.
        Node::Block { stmts, .. } => stmts.last().is_some_and(|last| is_tail_expr(last, fn_name)),

        // An expression-statement: the inner expression is in tail position.
        Node::ExpressionStatement { expr, .. } => is_tail_expr(expr, fn_name),

        // `return EXPR` — the expr is in tail position.
        Node::ReturnStatement {
            value: Some(inner), ..
        } => is_tail_expr(inner, fn_name),
        Node::ReturnStatement { value: None, .. } => false,

        // `if COND { THEN } else { ELSE }` — both branches are in tail position.
        // The condition is NOT in tail position.
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            is_tail_expr(consequence, fn_name)
                || alternative
                    .as_ref()
                    .is_some_and(|alt| is_tail_expr(alt, fn_name))
        }

        // `match SUBJECT { arm => expr, ... }` — each arm body is in tail position.
        Node::Match { arms, .. } => arms.iter().any(|(_, _, body)| is_tail_expr(body, fn_name)),

        // Everything else is not a tail call.
        _ => false,
    }
}

/// Collect every self-recursive call inside `body` that is NOT in tail
/// position. Returns the offending `CallExpression` nodes.
pub fn collect_non_tail_self_calls<'a>(body: &'a Node, fn_name: &str) -> Vec<&'a Node> {
    let mut out = Vec::new();
    collect_non_tail_impl(body, fn_name, /*in_tail=*/ true, &mut out);
    out
}

/// Recursive worker for `collect_non_tail_self_calls`.
///
/// `tail` indicates whether `node` is currently in a tail position. When a
/// `CallExpression` calling `fn_name` is encountered while `tail == false`,
/// it is a non-tail self-recursive call.
fn collect_non_tail_impl<'a>(node: &'a Node, fn_name: &str, tail: bool, out: &mut Vec<&'a Node>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let is_self_call = matches!(
                function.as_ref(),
                Node::Identifier { name, .. } if name == fn_name
            );
            if is_self_call && !tail {
                out.push(node);
            }
            // The arguments to any call are never in tail position.
            for arg in arguments {
                collect_non_tail_impl(arg, fn_name, false, out);
            }
            // The function-position expression (if not a plain identifier) is
            // not in tail position either.
            collect_non_tail_impl(function, fn_name, false, out);
        }

        Node::Block { stmts, .. } => {
            let len = stmts.len();
            for (i, stmt) in stmts.iter().enumerate() {
                let is_last = i + 1 == len;
                collect_non_tail_impl(stmt, fn_name, tail && is_last, out);
            }
        }

        Node::ExpressionStatement { expr, .. } => {
            collect_non_tail_impl(expr, fn_name, tail, out);
        }

        Node::ReturnStatement {
            value: Some(inner), ..
        } => {
            collect_non_tail_impl(inner, fn_name, tail, out);
        }
        Node::ReturnStatement { value: None, .. } => {}

        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            // Condition is never tail.
            collect_non_tail_impl(condition, fn_name, false, out);
            collect_non_tail_impl(consequence, fn_name, tail, out);
            if let Some(alt) = alternative {
                collect_non_tail_impl(alt, fn_name, tail, out);
            }
        }

        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_non_tail_impl(scrutinee, fn_name, false, out);
            for (_, _guard, body) in arms {
                collect_non_tail_impl(body, fn_name, tail, out);
            }
        }

        Node::LetStatement { value, .. } => {
            collect_non_tail_impl(value, fn_name, false, out);
        }

        Node::Assignment { value, .. } => {
            collect_non_tail_impl(value, fn_name, false, out);
        }

        Node::InfixExpression { left, right, .. } => {
            collect_non_tail_impl(left, fn_name, false, out);
            collect_non_tail_impl(right, fn_name, false, out);
        }

        Node::PrefixExpression { right, .. } => {
            collect_non_tail_impl(right, fn_name, false, out);
        }

        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_non_tail_impl(condition, fn_name, false, out);
            // Tail position does NOT propagate into a while body — the body
            // may execute many times.
            collect_non_tail_impl(body, fn_name, false, out);
        }

        Node::ForInStatement { iterable, body, .. } => {
            collect_non_tail_impl(iterable, fn_name, false, out);
            collect_non_tail_impl(body, fn_name, false, out);
        }

        Node::TryCatch { body, handlers, .. } => {
            // A try-catch introduces extra unwinding logic; treat the
            // whole block as non-tail (conservative).
            for stmt in body.iter() {
                collect_non_tail_impl(stmt, fn_name, false, out);
            }
            for (_, handler_body) in handlers {
                for stmt in handler_body.iter() {
                    collect_non_tail_impl(stmt, fn_name, false, out);
                }
            }
        }

        // Nodes that cannot directly contain a self-recursive call.
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// #[must_tail_call] check pass
// ---------------------------------------------------------------------------

/// RES-2592: validate `#[must_tail_call]` annotations.
///
/// For every function tagged with `#[must_tail_call]`, verify that every
/// self-recursive call is in tail position. Emit a compile error for each
/// non-tail self-recursive call found.
///
/// Called from `typechecker.rs` `<EXTENSION_PASSES>` via:
/// ```text
/// crate::tail_calls::check(program, source_path)?;
/// ```
pub fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let attrs = crate::feature_attrs::find_kind("must_tail_call");
    if attrs.is_empty() {
        return Ok(());
    }

    // Build a set of function names that carry #[must_tail_call].
    let annotated: std::collections::HashSet<&str> =
        attrs.iter().map(|(fn_name, _)| fn_name.as_str()).collect();

    let stmts = match program {
        Node::Program(s) => s,
        _ => return Ok(()),
    };

    let mut errors: Vec<String> = Vec::new();

    for spanned in stmts {
        if let Node::Function { name, body, .. } = &spanned.node {
            if !annotated.contains(name.as_str()) {
                continue;
            }
            let non_tail = collect_non_tail_self_calls(body, name);
            for bad_call in non_tail {
                let loc = call_span_fmt(bad_call, source_path);
                errors.push(format!(
                    "{}: `#[must_tail_call]` violation: self-recursive call to `{}` is not in tail position",
                    loc, name
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// Format a source location for a `CallExpression` node. Returns
/// `"<unknown>:?:?"` for nodes whose span we cannot recover.
fn call_span_fmt(node: &Node, source_path: &str) -> String {
    let file = if source_path.is_empty() {
        "<unknown>"
    } else {
        source_path
    };
    match node {
        Node::CallExpression { span, .. } => {
            format!("{}:{}:{}", file, span.start.line, span.start.column)
        }
        _ => format!("{}:?:?", file),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn parse_program(src: &str) -> Node {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        prog
    }

    fn body_of(program: &Node, fn_name: &str) -> Node {
        let stmts = match program {
            Node::Program(s) => s,
            _ => panic!("expected Program"),
        };
        for s in stmts {
            if let Node::Function { name, body, .. } = &s.node
                && name == fn_name
            {
                return body.as_ref().clone();
            }
        }
        panic!("function `{}` not found in program", fn_name)
    }

    // --- is_tail_call ---

    #[test]
    fn tail_call_in_simple_body() {
        // `fact(n - 1, acc * n)` is the last expression in the block.
        let src = "fn fact(int n, int acc) -> int { fact(n - 1, n * acc) }";
        let prog = parse_program(src);
        let body = body_of(&prog, "fact");
        assert!(is_tail_call(&body, "fact"));
    }

    #[test]
    fn tail_call_in_if_else() {
        let src = r#"
            fn f(int n) -> int {
                if n <= 0 { 0 } else { f(n - 1) }
            }
        "#;
        let prog = parse_program(src);
        let body = body_of(&prog, "f");
        assert!(is_tail_call(&body, "f"));
    }

    #[test]
    fn not_tail_call_in_arithmetic() {
        // `f(n-1) + 1` — the call is not in tail position.
        let src = r#"
            fn f(int n) -> int {
                if n <= 0 { 0 } else { f(n - 1) + 1 }
            }
        "#;
        let prog = parse_program(src);
        let body = body_of(&prog, "f");
        assert!(!is_tail_call(&body, "f"));
    }

    #[test]
    fn tail_call_not_detected_for_other_fn() {
        let src = "fn f(int n) -> int { g(n) }";
        let prog = parse_program(src);
        let body = body_of(&prog, "f");
        // Looking for self-call to `f`, but only `g(n)` is there.
        assert!(!is_tail_call(&body, "f"));
    }

    // --- collect_non_tail_self_calls ---

    #[test]
    fn non_tail_self_calls_detected() {
        // `1 + fact(n - 1, acc)` — the recursive call is not in tail position.
        let src = r#"
            fn fact(int n, int acc) -> int {
                if n <= 1 { acc } else { 1 + fact(n - 1, acc) }
            }
        "#;
        let prog = parse_program(src);
        let body = body_of(&prog, "fact");
        let bad = collect_non_tail_self_calls(&body, "fact");
        assert!(!bad.is_empty(), "expected at least one non-tail call");
    }

    #[test]
    fn tail_recursive_has_no_non_tail_calls() {
        let src = r#"
            fn fact(int n, int acc) -> int {
                if n <= 1 { acc } else { fact(n - 1, n * acc) }
            }
        "#;
        let prog = parse_program(src);
        let body = body_of(&prog, "fact");
        let bad = collect_non_tail_self_calls(&body, "fact");
        assert!(
            bad.is_empty(),
            "expected zero non-tail calls, got {}",
            bad.len()
        );
    }

    // --- check() pass ---

    #[test]
    fn check_passes_on_program_without_attribute() {
        // No #[must_tail_call] → check is a no-op.
        let src = r#"
            fn f(int n) -> int { if n == 0 { 0 } else { f(n - 1) + 1 } }
        "#;
        let prog = parse_program(src);
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_passes_on_properly_tail_recursive_fn() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = r#"
            #[must_tail_call]
            fn fact(int n, int acc) -> int {
                if n <= 1 { acc } else { fact(n - 1, n * acc) }
            }
        "#;
        let prog = parse_program(src);
        let result = check(&prog, "<test>");
        crate::feature_attrs::reset();
        assert!(result.is_ok(), "unexpected error: {:?}", result);
    }

    #[test]
    fn check_fails_on_non_tail_annotated_fn() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = r#"
            #[must_tail_call]
            fn bad_fn(int n) -> int {
                if n == 0 { 0 } else { bad_fn(n - 1) + 1 }
            }
        "#;
        let prog = parse_program(src);
        let result = check(&prog, "<test>");
        crate::feature_attrs::reset();
        assert!(result.is_err(), "expected error for non-tail annotated fn");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("must_tail_call"),
            "error should mention must_tail_call: {}",
            msg
        );
        assert!(
            msg.contains("`bad_fn`"),
            "error should mention function name: {}",
            msg
        );
    }
}
