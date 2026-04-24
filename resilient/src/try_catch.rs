//! RES-387 follow-up (RES-224): structured failure handlers.
//!
//! The MVP of RES-387 requires every `fails` variant declared on a
//! callee to be propagated on the caller's own `fails` list. This
//! module implements the other legal option: handling the failure at
//! the call site via a `try`/`catch` block.
//!
//! Syntax:
//!
//! ```text
//! try {
//!     callee(x);
//! } catch Timeout {
//!     recover();
//! } catch HardwareFault {
//!     log_fault();
//! }
//! ```
//!
//! Each `catch VariantName { ... }` arm subtracts that variant from
//! the propagation obligation inside the `try` body. A fully-handled
//! call (every declared failure caught) may appear in a caller with an
//! empty `fails` set; a partially-handled call still requires the
//! leftover variants on the caller's signature.
//!
//! This file owns:
//!
//! - [`parse`]: parser for the `try { ... } catch V { ... }` form.
//! - [`check`]: top-level program walk that validates each `catch`
//!   arm names a variant actually emitted inside the `try` body.
//!
//! The per-call-site subtraction (the part that makes an otherwise
//! unhandled variant acceptable) is implemented in the typechecker's
//! `Node::TryCatch` arm; see [`augmented_fn_fails`].
//!
//! Follow-ups tracked separately: binding the caught variant's
//! payload inside the handler body, and a `catch _` wildcard.

use crate::Node;
use crate::span::Span;

/// Sentinel inserted in place of a missing variant name when the
/// parser recovers from a malformed `catch` arm. Nothing in the
/// compiler ever emits a variant with an empty name, so the
/// typechecker's "unknown variant" branch will reject it cleanly.
const MISSING_VARIANT: &str = "";

/// RES-224: parse a `try { ... } catch V { ... } (catch V { ... })*`
/// statement. On entry, `current_token` must be `Token::Try`; on exit
/// the parser cursor sits on the closing `}` of the last handler
/// block (matching how every other block-statement parser leaves the
/// cursor — the outer `parse_block_statement` advances past it).
///
/// The return value is always a `Node::TryCatch`. Parse errors are
/// accumulated on the parser via `record_error`; on an unrecoverable
/// error the node still comes out with whatever was parsed so far so
/// the rest of the program can continue type-checking.
pub(crate) fn parse(parser: &mut crate::Parser) -> Node {
    let stmt_span = parser.span_at_current();
    // Consume `try`.
    parser.next_token();

    // Parse the try-body. On exit the cursor sits on the closing `}`.
    // Only advance past it if the next token is `catch` — otherwise
    // leaving the cursor on `}` keeps us compatible with the outer
    // dispatch loop's single advance.
    let body = parse_block(parser, "try");
    if parser.current_token == crate::Token::RightBrace && parser.peek_token == crate::Token::Catch
    {
        parser.next_token();
    }

    let mut handlers: Vec<(String, Vec<Node>)> = Vec::new();
    while parser.current_token == crate::Token::Catch {
        parser.next_token(); // consume `catch`
        let variant = match &parser.current_token {
            crate::Token::Identifier(n) => {
                let name = n.clone();
                parser.next_token();
                name
            }
            other => {
                let tok = other.clone();
                parser.record_error(format!(
                    "Expected failure-variant identifier after `catch`, found {}",
                    tok
                ));
                MISSING_VARIANT.to_string()
            }
        };
        let handler_body = parse_block(parser, "catch");
        handlers.push((variant, handler_body));
        // Peek past this handler's `}` to see if another `catch`
        // follows. If the next non-trivial token is NOT `catch` we
        // must NOT advance — leaving the cursor on `}` matches the
        // convention every other block-statement parser uses, so the
        // outer dispatch loop in `parse_block_statement` can do its
        // single `next_token()` without skipping a top-level stmt.
        if parser.peek_token == crate::Token::Catch {
            parser.next_token();
        } else {
            break;
        }
    }

    if handlers.is_empty() {
        parser.record_error(
            "`try { ... }` must be followed by at least one `catch VariantName { ... }` arm"
                .to_string(),
        );
    }

    Node::TryCatch {
        span: stmt_span,
        body,
        handlers,
    }
}

/// Parse a `{ stmt; stmt; ... }` block and return the inner
/// statements. Leaves the parser cursor on the closing `}` — matching
/// `parse_block_statement`'s convention so the dispatch loop in
/// `parse_block_statement` can advance once to skip it.
fn parse_block(parser: &mut crate::Parser, context: &str) -> Vec<Node> {
    if parser.current_token != crate::Token::LeftBrace {
        let tok = parser.current_token.clone();
        parser.record_error(format!("Expected '{{' after `{}`, found {}", context, tok));
        return Vec::new();
    }
    let mut stmts: Vec<Node> = Vec::new();
    parser.next_token(); // skip `{`
    while parser.current_token != crate::Token::RightBrace
        && parser.current_token != crate::Token::Eof
    {
        if let Some(stmt) = parser.parse_statement() {
            stmts.push(stmt);
        }
        parser.next_token();
    }
    stmts
}

/// Compute the `fails` set in scope for statements nested inside a
/// `try { ... } catch V { ... }` block. Returns the outer scope's
/// declared variants plus every variant named by a `catch` arm.
///
/// The typechecker swaps its `current_fn_fails` field to this vector
/// before walking the try-body, so an otherwise-unhandled call inside
/// the body type-checks as long as the caught variants cover it.
pub(crate) fn augmented_fn_fails(
    outer: Option<&Vec<String>>,
    handlers: &[(String, Vec<Node>)],
) -> Vec<String> {
    let mut combined: Vec<String> = outer.cloned().unwrap_or_default();
    for (variant, _) in handlers {
        if !combined.iter().any(|v| v == variant) {
            combined.push(variant.clone());
        }
    }
    combined
}

/// RES-224 program-level pass. Validates that every `catch` arm
/// references a failure variant that some callee inside the matching
/// `try` body actually emits — a `catch Ghost` arm that will never
/// fire would silently allow a broken propagation obligation, so it's
/// a hard error.
///
/// Walks every top-level fn body (and every nested block / try-catch)
/// and checks each TryCatch node in-place. Errors carry the
/// `source_path:line:col: ` prefix the rest of the typechecker uses.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let Node::Program(statements) = program else {
        return Ok(());
    };
    // Collect the table of each fn's declared `fails` set so we can
    // resolve call sites inside try bodies without round-tripping
    // through the main typechecker.
    let mut fn_fails: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for stmt in statements {
        if let Node::Function { name, fails, .. } = &stmt.node {
            fn_fails.insert(name.clone(), fails.clone());
        }
    }
    for stmt in statements {
        if let Node::Function { body, .. } = &stmt.node {
            walk(body, &fn_fails, source_path)?;
        }
    }
    Ok(())
}

/// Walk an AST subtree and validate every `TryCatch` encountered.
fn walk(
    node: &Node,
    fn_fails: &std::collections::HashMap<String, Vec<String>>,
    source_path: &str,
) -> Result<(), String> {
    match node {
        Node::TryCatch {
            span,
            body,
            handlers,
        } => {
            let emitted = collect_emitted_variants(body, fn_fails);
            for (variant, handler_body) in handlers {
                if variant.is_empty() {
                    // Parser already emitted a recovery diagnostic.
                    continue;
                }
                if !emitted.iter().any(|v| v == variant) {
                    return Err(format_error(
                        source_path,
                        span,
                        format!(
                            "`catch {}` arm does not correspond to any failure variant emitted inside the `try` body",
                            variant
                        ),
                    ));
                }
                for stmt in handler_body {
                    walk(stmt, fn_fails, source_path)?;
                }
            }
            for stmt in body {
                walk(stmt, fn_fails, source_path)?;
            }
            Ok(())
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk(s, fn_fails, source_path)?;
            }
            Ok(())
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            walk(consequence, fn_fails, source_path)?;
            if let Some(alt) = alternative {
                walk(alt, fn_fails, source_path)?;
            }
            Ok(())
        }
        Node::WhileStatement { body, .. }
        | Node::ForInStatement { body, .. }
        | Node::LiveBlock { body, .. } => walk(body, fn_fails, source_path),
        _ => Ok(()),
    }
}

/// Gather the set of failure variants any call inside `body` could
/// produce, based on the statically-known `fails` declaration of each
/// callee. Calls to unknown (non-user-declared) functions contribute
/// nothing — those never carry a `fails` obligation in the current
/// MVP, so a `catch` arm covering them would always be spurious.
fn collect_emitted_variants(
    body: &[Node],
    fn_fails: &std::collections::HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for stmt in body {
        collect_from_node(stmt, fn_fails, &mut out);
    }
    out
}

fn collect_from_node(
    node: &Node,
    fn_fails: &std::collections::HashMap<String, Vec<String>>,
    out: &mut Vec<String>,
) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref()
                && let Some(variants) = fn_fails.get(name)
            {
                for v in variants {
                    if !out.iter().any(|x| x == v) {
                        out.push(v.clone());
                    }
                }
            }
            for arg in arguments {
                collect_from_node(arg, fn_fails, out);
            }
        }
        Node::ExpressionStatement { expr, .. } => collect_from_node(expr, fn_fails, out),
        Node::ReturnStatement { value: Some(v), .. } => collect_from_node(v, fn_fails, out),
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Assignment { value, .. } => collect_from_node(value, fn_fails, out),
        Node::InfixExpression { left, right, .. } => {
            collect_from_node(left, fn_fails, out);
            collect_from_node(right, fn_fails, out);
        }
        Node::PrefixExpression { right, .. } => collect_from_node(right, fn_fails, out),
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_from_node(s, fn_fails, out);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_from_node(condition, fn_fails, out);
            collect_from_node(consequence, fn_fails, out);
            if let Some(alt) = alternative {
                collect_from_node(alt, fn_fails, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_from_node(condition, fn_fails, out);
            collect_from_node(body, fn_fails, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_from_node(iterable, fn_fails, out);
            collect_from_node(body, fn_fails, out);
        }
        Node::TryCatch { body, handlers, .. } => {
            // Variants caught by inner try/catch do NOT propagate out
            // — the outer try should not see them unless some other
            // call in its own body also emits them.
            let inner_emitted = collect_emitted_variants(body, fn_fails);
            let caught: std::collections::HashSet<&str> =
                handlers.iter().map(|(v, _)| v.as_str()).collect();
            for v in &inner_emitted {
                if !caught.contains(v.as_str()) && !out.iter().any(|x| x == v) {
                    out.push(v.clone());
                }
            }
            for (_, hbody) in handlers {
                for s in hbody {
                    collect_from_node(s, fn_fails, out);
                }
            }
        }
        _ => {}
    }
}

/// Format an error with the usual `source_path:line:col: ` prefix,
/// matching how the typechecker prefixes its own diagnostics.
fn format_error(source_path: &str, span: &Span, msg: String) -> String {
    if span.start.line == 0 {
        msg
    } else {
        format!(
            "{}:{}:{}: {}",
            source_path, span.start.line, span.start.column, msg
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse as parse_program;
    use crate::typechecker::TypeChecker;

    fn check_src(src: &str) -> Result<(), String> {
        let (prog, errs) = parse_program(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = TypeChecker::new();
        tc.check_program_with_source(&prog, "<t>").map(|_| ())
    }

    #[test]
    fn parser_accepts_single_catch_arm() {
        let src = "\
            fn risky(int x) fails Timeout { return x; }\n\
            fn caller(int y) {\n\
                try { risky(y); } catch Timeout { return; }\n\
            }\n";
        let (prog, errs) = parse_program(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        // Walk down to the TryCatch node and confirm its shape.
        let mut found = false;
        if let Node::Program(stmts) = &prog {
            for stmt in stmts {
                if let Node::Function { body, .. } = &stmt.node
                    && let Node::Block { stmts, .. } = body.as_ref()
                {
                    for s in stmts {
                        if let Node::TryCatch { handlers, .. } = s {
                            assert_eq!(handlers.len(), 1);
                            assert_eq!(handlers[0].0, "Timeout");
                            found = true;
                        }
                    }
                }
            }
        }
        assert!(found, "expected a TryCatch node in the parsed program");
    }

    #[test]
    fn exhaustive_handler_accepted_with_empty_caller_fails() {
        // Caller declares no `fails` — the try block fully handles
        // the callee's single variant.
        let src = "\
            fn risky(int x) fails Timeout { return x; }\n\
            fn caller(int y) {\n\
                try { risky(y); } catch Timeout { return; }\n\
            }\n";
        check_src(src).expect("exhaustive handler should typecheck");
    }

    #[test]
    fn partial_handler_requires_leftover_on_caller() {
        // Callee emits two variants; caller only catches one. The
        // leftover MUST appear on caller's `fails` list — if it does
        // not, the unhandled-variant diagnostic must fire.
        let src_bad = "\
            fn risky(int x) fails A, B { return x; }\n\
            fn caller(int y) {\n\
                try { risky(y); } catch A { return; }\n\
            }\n";
        let err = check_src(src_bad).expect_err("partial handler must require leftover");
        assert!(
            err.contains("unhandled failure variant B"),
            "unexpected error: {err}"
        );

        // With the leftover declared, the caller typechecks.
        let src_ok = "\
            fn risky(int x) fails A, B { return x; }\n\
            fn caller(int y) fails B {\n\
                try { risky(y); } catch A { return; }\n\
            }\n";
        check_src(src_ok).expect("partial handler with leftover must typecheck");
    }

    #[test]
    fn unhandled_variant_diagnostic_mentions_catch() {
        // The plain propagation error (no try/catch in play) must
        // still point at `catch` as a valid resolution option.
        let src = "\
            fn risky(int x) fails Timeout { return x; }\n\
            fn caller(int y) { return risky(y); }\n";
        let err = check_src(src).expect_err("unhandled variant must error");
        assert!(
            err.contains("catch"),
            "diagnostic should mention `catch` as a resolution: {err}"
        );
    }

    #[test]
    fn catch_arm_for_unemitted_variant_is_rejected() {
        // The `try` body emits only `Timeout`, but the handler catches
        // `Ghost` — a dead arm that would mask a broken propagation
        // obligation if accepted.
        let src = "\
            fn risky(int x) fails Timeout { return x; }\n\
            fn caller(int y) fails Timeout {\n\
                try { risky(y); } catch Ghost { return; }\n\
            }\n";
        let err = check_src(src).expect_err("dead catch arm must be rejected");
        assert!(err.contains("`catch Ghost`"), "unexpected error: {err}");
    }

    #[test]
    fn multi_variant_fully_handled() {
        let src = "\
            fn risky(int x) fails A, B { return x; }\n\
            fn caller(int y) {\n\
                try { risky(y); } catch A { return; } catch B { return; }\n\
            }\n";
        check_src(src).expect("exhaustive multi-variant handler should typecheck");
    }
}
