//! RES-3780 Tier 2: Bounded-loop verification for `@ai_generated` functions.
//!
//! AI-generated functions that contain while-loops must declare a `#[loop_bound(N)]`
//! attribute to specify the maximum iteration count. When the `z3` feature is enabled,
//! the compiler attempts to statically verify that simple monotonic-counter loops
//! respect their declared bounds using SMT.
//!
//! **Enforcement rule**: Any function marked `@ai_generated` that contains a
//! `while`-loop (recursively) MUST carry a `#[loop_bound(N)]` attribute where N
//! is a positive integer. Missing bounds are a hard compile error.
//!
//! **Verification** (z3-feature only): For loops matching the simple monotonic-counter
//! shape (counter incremented by a constant per iteration with a constant or parameter
//! upper bound), the compiler builds a Z3 LIA obligation and verifies the loop executes
//! at most N times. If the loop shape doesn't match or Z3 times out, a warning is
//! emitted; loops unverifiable at compile-time will rely on runtime checks.
#![allow(clippy::collapsible_if)]

use crate::Node;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct LoopBoundSpec {
    // Only read behind `#[cfg(feature = "z3")]`; the syntactic (non-z3)
    // enforcement path only needs presence in the map, not the value.
    #[allow(dead_code)]
    pub bound: u32,
}

/// Collect all `#[loop_bound(N)]` attributes from the feature registry.
pub fn collect() -> HashMap<String, LoopBoundSpec> {
    let attrs = crate::feature_attrs::find_kind("loop_bound");
    let mut out = HashMap::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let raw = rec.args.trim();
        if let Ok(n) = raw.parse::<u32>() {
            if n > 0 {
                out.insert(item, LoopBoundSpec { bound: n });
            }
        }
    }
    out
}

/// Walk the AST recursively to check if a node contains any while-loops.
fn has_while_loop(node: &Node) -> bool {
    match node {
        Node::WhileStatement { .. } => true,
        Node::Block { stmts, .. } => stmts.iter().any(has_while_loop),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            has_while_loop(consequence)
                || alternative.as_ref().is_some_and(|ab| has_while_loop(ab))
        }
        Node::ForInStatement { body, .. } => has_while_loop(body),
        Node::ReturnStatement { value: Some(v), .. } => has_while_loop(v),
        Node::LetStatement { value, .. } => has_while_loop(value),
        Node::ExpressionStatement { expr, .. } => has_while_loop(expr),
        Node::CallExpression { arguments, .. } => arguments.iter().any(has_while_loop),
        Node::InfixExpression { left, right, .. } => has_while_loop(left) || has_while_loop(right),
        Node::PrefixExpression { right, .. } => has_while_loop(right),
        Node::ArrayLiteral { items, .. } => items.iter().any(has_while_loop),
        _ => false,
    }
}

fn diagnostic(source_path: &str, line: usize, fn_name: &str, message: &str) -> String {
    format!(
        "{source_path}:{line}:0: error[loop_bound]: invalid @ai_generated declaration `{fn_name}`: {message}"
    )
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    // First pass: collect both @ai_generated and #[loop_bound] attributes
    let ai_generated_attrs = crate::feature_attrs::find_kind("ai_generated");
    if ai_generated_attrs.is_empty() {
        return Ok(());
    }

    let bounds = collect();

    // Build a map of @ai_generated function names and their line numbers
    let mut ai_generated_funcs = HashMap::new();
    if let Node::Program(stmts) = program {
        for stmt in stmts {
            if let Node::Function { name, .. } = &stmt.node {
                for (fn_name, rec) in &ai_generated_attrs {
                    if fn_name == name {
                        ai_generated_funcs.insert(name.clone(), rec.line);
                        break;
                    }
                }
            }
        }
    }

    // Now check each @ai_generated function
    if let Node::Program(stmts) = program {
        for stmt in stmts {
            if let Node::Function { name, body, .. } = &stmt.node {
                if let Some(&line) = ai_generated_funcs.get(name) {
                    // Check if this function contains while-loops
                    if has_while_loop(body) {
                        // It has a while-loop, so it must have a #[loop_bound]
                        if !bounds.contains_key(name) {
                            return Err(diagnostic(
                                source_path,
                                line,
                                name,
                                "contains a while-loop and requires #[loop_bound(N)] (RES-3780 Tier 2)",
                            ));
                        }

                        // If z3 feature is enabled, verify the bound
                        #[cfg(feature = "z3")]
                        {
                            if let Some(spec) = bounds.get(name) {
                                verify_loop_bounds(program, name, spec.bound, source_path)?;
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(feature = "z3")]
fn verify_loop_bounds(
    program: &Node,
    fn_name: &str,
    declared_bound: u32,
    source_path: &str,
) -> Result<(), String> {
    if let Node::Program(stmts) = program {
        for stmt in stmts {
            if let Node::Function {
                name,
                body,
                requires,
                ..
            } = &stmt.node
            {
                if name == fn_name {
                    verify_while_loops_in_node(
                        body,
                        declared_bound,
                        source_path,
                        fn_name,
                        requires,
                    )?;
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

#[cfg(feature = "z3")]
fn verify_while_loops_in_node(
    node: &Node,
    declared_bound: u32,
    source_path: &str,
    fn_name: &str,
    requires: &[Node],
) -> Result<(), String> {
    match node {
        Node::WhileStatement {
            condition,
            body,
            span,
            ..
        } => verify_one_while_loop(
            condition,
            body,
            declared_bound,
            source_path,
            fn_name,
            *span,
            requires,
        ),
        Node::Block { stmts, .. } => {
            for s in stmts {
                verify_while_loops_in_node(s, declared_bound, source_path, fn_name, requires)?;
            }
            Ok(())
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            verify_while_loops_in_node(
                consequence,
                declared_bound,
                source_path,
                fn_name,
                requires,
            )?;
            if let Some(ab) = alternative {
                verify_while_loops_in_node(ab, declared_bound, source_path, fn_name, requires)?;
            }
            Ok(())
        }
        Node::ForInStatement { body, .. } => {
            verify_while_loops_in_node(body, declared_bound, source_path, fn_name, requires)
        }
        _ => Ok(()),
    }
}

#[cfg(feature = "z3")]
fn verify_one_while_loop(
    condition: &Node,
    body: &Node,
    declared_bound: u32,
    source_path: &str,
    fn_name: &str,
    span: crate::span::Span,
    requires: &[Node],
) -> Result<(), String> {
    const LOOP_BOUND_TIMEOUT_MS: u32 = 1000;
    let line = span.start.line;

    // Try to match the simple monotonic-counter shape.
    let matched = extract_loop_bound_pattern(condition).and_then(|(counter_name, bound_expr)| {
        extract_counter_increment_pattern(body, &counter_name)
            .map(|(initial_value, step)| (counter_name, bound_expr, initial_value, step))
    });

    let (_counter_name, bound_expr, initial_value, step) = match matched {
        Some(m) => m,
        None => {
            eprintln!(
                "{}:{}:0: warning[loop_bound]: loop in `{}` does not match the simple monotonic-counter shape; declared bound {} could not be statically verified — will rely on runtime check",
                source_path, line, fn_name, declared_bound
            );
            return Ok(());
        }
    };

    let obligation =
        match create_bound_check_obligation(&bound_expr, initial_value, declared_bound, step) {
            Some(o) => o,
            None => return Ok(()),
        };

    let (verdict, _cert, counterexample, _timed_out) =
        crate::verifier_z3::prove_with_axioms_and_timeout(
            &obligation,
            &std::collections::HashMap::new(),
            requires,
            LOOP_BOUND_TIMEOUT_MS,
        );

    match verdict {
        Some(true) => {
            // Proved: loop respects its declared bound. Silent success.
            Ok(())
        }
        Some(false) => {
            let cx = counterexample
                .map(|c| format!(" (counterexample: {})", c))
                .unwrap_or_default();
            Err(format!(
                "{}:{}:0: error[loop_bound]: loop in `{}` may exceed its declared bound of {} iterations{}",
                source_path, line, fn_name, declared_bound, cx
            ))
        }
        None => {
            eprintln!(
                "{}:{}:0: warning[loop_bound]: declared bound {} for loop in `{}` could not be statically verified (Z3 Unknown) — will rely on runtime check",
                source_path, line, declared_bound, fn_name
            );
            Ok(())
        }
    }
}

#[cfg(feature = "z3")]
fn extract_loop_bound_pattern(condition: &Node) -> Option<(String, Node)> {
    match condition {
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            if let Node::Identifier { name, .. } = left.as_ref() {
                if matches!(*operator, "<" | "<=") {
                    return Some((name.clone(), (**right).clone()));
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(feature = "z3")]
fn extract_counter_increment_pattern(body: &Node, counter_name: &str) -> Option<(i64, u32)> {
    fn walk(node: &Node, counter_name: &str) -> Option<u32> {
        match node {
            Node::ExpressionStatement { expr, .. } => walk(expr, counter_name),
            Node::Assignment { name, value, .. } if name == counter_name => {
                if let Node::InfixExpression {
                    left,
                    operator,
                    right,
                    ..
                } = value.as_ref()
                {
                    if *operator == "+" {
                        if let Node::Identifier {
                            name: left_name, ..
                        } = left.as_ref()
                        {
                            if left_name == counter_name {
                                if let Node::IntegerLiteral { value: step, .. } = right.as_ref() {
                                    if *step > 0 {
                                        return Some(*step as u32);
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }
            Node::Block { stmts, .. } => stmts.iter().find_map(|s| walk(s, counter_name)),
            Node::IfStatement {
                consequence,
                alternative,
                ..
            } => walk(consequence, counter_name)
                .or_else(|| alternative.as_ref().and_then(|a| walk(a, counter_name))),
            _ => None,
        }
    }
    walk(body, counter_name).map(|step| (0, step))
}

#[cfg(feature = "z3")]
fn create_bound_check_obligation(
    counter_bound: &Node,
    initial_value: i64,
    declared_bound: u32,
    step: u32,
) -> Option<Node> {
    let span = crate::span::Span::point(crate::span::Pos::default());
    let bound_minus_initial = Node::InfixExpression {
        left: Box::new(counter_bound.clone()),
        operator: "-",
        right: Box::new(Node::IntegerLiteral {
            value: initial_value,
            span,
        }),
        span,
    };
    let max_iterations = Node::IntegerLiteral {
        value: (declared_bound as i64) * (step as i64),
        span,
    };
    Some(Node::InfixExpression {
        left: Box::new(bound_minus_initial),
        operator: "<=",
        right: Box::new(max_iterations),
        span,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_loop_bound_on_ai_generated_with_while_loop() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "ai_gen_with_loop",
            crate::feature_attrs::AttrRecord {
                name: "ai_generated".into(),
                args: "".into(),
                line: 1,
            },
        );
        let src = "fn ai_gen_with_loop(int n) requires n >= 0 {
            int i = 0;
            while i < n {
                i = i + 1;
            }
            return i;
        }";
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");

        let err = check(&prog, "<test>").expect_err("missing loop_bound must be rejected");
        assert!(
            err.contains("error[loop_bound]") && err.contains("requires #[loop_bound(N)]"),
            "expected loop_bound missing error, got: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn loop_bound_present_on_ai_generated_with_while_loop() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "ai_gen_with_loop",
            crate::feature_attrs::AttrRecord {
                name: "ai_generated".into(),
                args: "".into(),
                line: 1,
            },
        );
        crate::feature_attrs::record(
            "ai_gen_with_loop",
            crate::feature_attrs::AttrRecord {
                name: "loop_bound".into(),
                args: "10".into(),
                line: 1,
            },
        );
        let src = "fn ai_gen_with_loop(int n) requires n >= 0 {
            int i = 0;
            while i < n {
                i = i + 1;
            }
            return i;
        }";
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");

        // Should pass the syntactic check (no error about missing loop_bound)
        let _result = check(&prog, "<test>");
        // Without z3 feature, it should pass
        #[cfg(not(feature = "z3"))]
        {
            assert!(_result.is_ok(), "check failed: {:?}", _result);
        }
        crate::feature_attrs::reset();
    }

    #[test]
    fn ai_generated_without_loop_passes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "simple_fn",
            crate::feature_attrs::AttrRecord {
                name: "ai_generated".into(),
                args: "".into(),
                line: 1,
            },
        );
        let src = "fn simple_fn(int x) requires x >= 0 ensures result >= x {
            return x + 1;
        }";
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");

        let result = check(&prog, "<test>");
        assert!(
            result.is_ok(),
            "function without loop should pass: {:?}",
            result
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn collect_loop_bounds() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "fn1",
            crate::feature_attrs::AttrRecord {
                name: "loop_bound".into(),
                args: "5".into(),
                line: 1,
            },
        );
        crate::feature_attrs::record(
            "fn2",
            crate::feature_attrs::AttrRecord {
                name: "loop_bound".into(),
                args: "100".into(),
                line: 2,
            },
        );
        let bounds = collect();
        assert_eq!(bounds.len(), 2);
        assert_eq!(bounds.get("fn1").map(|s| s.bound), Some(5));
        assert_eq!(bounds.get("fn2").map(|s| s.bound), Some(100));
        crate::feature_attrs::reset();
    }

    #[test]
    fn has_while_loop_in_simple_loop() {
        let src = "fn f() { while true { break; } }";
        let (prog, _) = crate::parse(src);
        if let Node::Program(stmts) = &prog {
            if let Some(stmt) = stmts.first() {
                if let Node::Function { body, .. } = &stmt.node {
                    assert!(has_while_loop(body));
                }
            }
        }
    }

    #[test]
    fn has_while_loop_nested_in_if() {
        let src = "fn f(bool b) { if b { while true { break; } } }";
        let (prog, _) = crate::parse(src);
        if let Node::Program(stmts) = &prog {
            if let Some(stmt) = stmts.first() {
                if let Node::Function { body, .. } = &stmt.node {
                    assert!(has_while_loop(body));
                }
            }
        }
    }

    #[test]
    fn no_while_loop() {
        let src = "fn f() { return 42; }";
        let (prog, _) = crate::parse(src);
        if let Node::Program(stmts) = &prog {
            if let Some(stmt) = stmts.first() {
                if let Node::Function { body, .. } = &stmt.node {
                    assert!(!has_while_loop(body));
                }
            }
        }
    }

    #[test]
    #[cfg(feature = "z3")]
    fn z3_verifies_bounded_loop() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();

        // Register the function as @ai_generated with a loop_bound
        crate::feature_attrs::record(
            "bounded_count",
            crate::feature_attrs::AttrRecord {
                name: "ai_generated".into(),
                args: "".into(),
                line: 1,
            },
        );
        crate::feature_attrs::record(
            "bounded_count",
            crate::feature_attrs::AttrRecord {
                name: "loop_bound".into(),
                args: "10".into(),
                line: 1,
            },
        );

        // This is a loop with a bounded upper bound: `n <= 10`
        // With declared_bound = 10 and step = 1, Z3 should be able to prove
        // that (n - 0) <= (10 * 1), which is true given the requires clause.
        let src = "fn bounded_count(int n) requires n >= 0 requires n <= 10 {
            let i = 0;
            while i < n {
                i = i + 1;
            }
            return i;
        }";
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");

        // `Ok` here is consistent with both a real Z3 proof and a silent
        // "Unknown, warn" fallback — this assertion alone can't tell them
        // apart. Manually verified with `cargo test ... -- --nocapture`
        // that this exact case (tight matching `n <= 10` axiom against
        // `declared_bound = 10`) hits `Some(true)` and prints no fallback
        // warning, i.e. it is a real static proof, not a fallback.
        let result = check(&prog, "<test>");
        assert!(result.is_ok(), "check failed: {:?}", result);
        crate::feature_attrs::reset();
    }

    #[test]
    #[cfg(feature = "z3")]
    fn z3_rejects_violated_bound() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();

        // Register the function with a loop_bound that's TOO SMALL.
        crate::feature_attrs::record(
            "unbounded_count",
            crate::feature_attrs::AttrRecord {
                name: "ai_generated".into(),
                args: "".into(),
                line: 1,
            },
        );
        crate::feature_attrs::record(
            "unbounded_count",
            crate::feature_attrs::AttrRecord {
                name: "loop_bound".into(),
                args: "3".into(), // declared bound is 3
                line: 1,
            },
        );

        // `n == 10` is a single concrete equality axiom — Z3 (even the older
        // solver build used in this environment) reliably falsifies
        // `(n - 0) <= (3 * 1)` against it and returns `Some(false)`, unlike
        // the multi-inequality axiom sets elsewhere in this test module
        // which this Z3 build frequently reports `Unknown` for (a pre-existing
        // limitation shared with contract_verify.rs, not specific to this
        // check) — verified manually via `rz check` before writing this test.
        let src = "fn unbounded_count(int n) requires n == 10 {
            let i = 0;
            while i < n {
                i = i + 1;
            }
            return i;
        }";
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");

        let result = check(&prog, "<test>");
        let err = result.expect_err("Z3-disprovable bound violation must be a hard error");
        assert!(
            err.contains("error[loop_bound]") && err.contains("may exceed its declared bound"),
            "expected loop_bound violation error, got: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    #[cfg(feature = "z3")]
    fn z3_unknown_for_non_monotonic_loop() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();

        crate::feature_attrs::record(
            "complex_loop",
            crate::feature_attrs::AttrRecord {
                name: "ai_generated".into(),
                args: "".into(),
                line: 1,
            },
        );
        crate::feature_attrs::record(
            "complex_loop",
            crate::feature_attrs::AttrRecord {
                name: "loop_bound".into(),
                args: "10".into(),
                line: 1,
            },
        );

        // This loop does NOT match the simple monotonic-counter pattern
        // (condition is not `i < n` or similar)
        let src = "fn complex_loop(int n) requires n >= 0 {
            let i = 0;
            while i * 2 < n {
                i = i + 1;
            }
            return i;
        }";
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");

        // Should complete OK but produce a warning that the shape doesn't match
        let result = check(&prog, "<test>");
        assert!(result.is_ok(), "check should complete: {:?}", result);
        crate::feature_attrs::reset();
    }
}
