//! RES-2659: Tail call optimization for mutually-recursive functions.
//!
//! When two (or more) functions annotated with `#[mutual_tail_call]` call
//! each other, the interpreter uses a trampoline to avoid growing the Rust
//! call stack. Without this, deep mutual recursion (e.g. even/odd on large
//! inputs, CPS state machines) exhausts the stack on embedded targets.
//!
//! ## Usage
//!
//! ```text
//! #[mutual_tail_call]
//! fn is_even(Int n) -> bool {
//!     if (n == 0) { return true; }
//!     return is_odd(n - 1);
//! }
//!
//! #[mutual_tail_call]
//! fn is_odd(Int n) -> bool {
//!     if (n == 0) { return false; }
//!     return is_even(n - 1);
//! }
//! ```
//!
//! Both functions in the pair (or group) must carry `#[mutual_tail_call]`.
//! The cross-function call in each function must be the last operation
//! before the function returns (tail position).
//!
//! ## Implementation
//!
//! All logic lives here. The core files add only:
//!
//! - `feature_attrs.rs`: `"mutual_tail_call"` in `is_known_attribute`.
//! - `lib.rs Value`: `MutualTailCall { callee: String, args: Vec<Value> }` variant.
//! - `lib.rs Interpreter`: `mutual_tco_fn: Option<String>` field.
//! - `lib.rs` eval (CallExpression): emit `Value::MutualTailCall` when callee
//!   is `#[mutual_tail_call]` and current fn is in mutual-TCO mode.
//! - `lib.rs` apply_function: trampoline handles `MutualTailCall` by re-dispatching.
//! - `typechecker.rs` `<EXTENSION_PASSES>`: `crate::mutual_tco::check(...)`.
//!
//! ## Trampoline mechanics
//!
//! `apply_function` creates a child interpreter and a `'tco` loop. When the
//! loop sees `Value::MutualTailCall { callee, args }` it:
//!
//! 1. Looks up the callee in the environment (`Value::Function`).
//! 2. Sets the callee's parameters in the env.
//! 3. Updates `mutual_tco_fn` on the child interpreter.
//! 4. Evaluates the callee's body — no new Rust call frame.
//!
//! Self-recursive calls inside a `#[mutual_tail_call]` function still work via
//! `Value::TailCall` (the existing mechanism) — they just rebind the same
//! parameters and restart the same body.

use crate::{Node, feature_attrs};

/// Returns `true` when `fn_name` is annotated with `#[mutual_tail_call]`.
///
/// Uses the same `find_kind` fast-path as `tail_calls::is_must_tail_call` —
/// an atomic load rejects the common case (no annotations) without scanning.
pub(crate) fn is_mutual_tail_call(fn_name: &str) -> bool {
    let attrs = feature_attrs::find_kind("mutual_tail_call");
    attrs.iter().any(|(name, _)| name == fn_name)
}

/// Typecheck pass: validate `#[mutual_tail_call]` usage.
///
/// Currently checks that each annotated function calls at least one other
/// `#[mutual_tail_call]` function, catching the degenerate case where a
/// programmer annotates a function but forgets to annotate its partner.
///
/// Full call-graph cycle validation (ensuring calls are in tail position) is
/// a follow-up under RES-2659 acceptance criteria.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let attrs = feature_attrs::find_kind("mutual_tail_call");
    if attrs.is_empty() {
        return Ok(());
    }

    // Collect all function names annotated with #[mutual_tail_call].
    // Attributes are tracked by feature_attrs, not stored in Node::Function.
    let annotated: std::collections::HashSet<&str> =
        attrs.iter().map(|(name, _)| name.as_str()).collect();

    let Node::Program(stmts) = program else {
        return Ok(());
    };

    for stmt in stmts {
        let Node::Function { name, body, .. } = &stmt.node else {
            continue;
        };

        if !annotated.contains(name.as_str()) {
            continue;
        }

        // Check that the body contains at least one call to another
        // #[mutual_tail_call]-annotated function. This catches the common
        // mistake of annotating only one side of the mutual recursion.
        if !body_calls_any(body, &annotated, name.as_str()) {
            return Err(format!(
                "{}: error: `#[mutual_tail_call]` function `{}` does not call any other \
                 `#[mutual_tail_call]`-annotated function — both sides of a mutual recursion \
                 group must carry the attribute",
                source_path, name
            ));
        }
    }
    Ok(())
}

/// Walk `node` (the body of a `#[mutual_tail_call]` function) and return
/// `true` if it directly calls any function in `annotated` other than `self_name`.
fn body_calls_any(
    node: &Node,
    annotated: &std::collections::HashSet<&str>,
    self_name: &str,
) -> bool {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref()
                && name != self_name
                && annotated.contains(name.as_str())
            {
                return true;
            }
            // Also check arguments for nested calls.
            arguments
                .iter()
                .any(|a| body_calls_any(a, annotated, self_name))
                || body_calls_any(function, annotated, self_name)
        }
        // Node::Block stmts is Vec<Node> (not Spanned).
        Node::Block { stmts, .. } => stmts
            .iter()
            .any(|s| body_calls_any(s, annotated, self_name)),
        Node::ReturnStatement { value: Some(e), .. } => body_calls_any(e, annotated, self_name),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            body_calls_any(condition, annotated, self_name)
                || body_calls_any(consequence, annotated, self_name)
                || alternative
                    .as_deref()
                    .is_some_and(|e| body_calls_any(e, annotated, self_name))
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            body_calls_any(scrutinee, annotated, self_name)
                || arms
                    .iter()
                    .any(|(_, _, body)| body_calls_any(body, annotated, self_name))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse, run_program};

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    fn parse_program(src: &str) -> Node {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        prog
    }

    #[test]
    fn even_odd_small() {
        // Hold the test lock so reset() in other tests can't race the registry.
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        // Basic mutual recursion — 10 is small enough to work without TCO too.
        let r = run(r#"
#[mutual_tail_call]
fn is_even(Int n) -> bool {
    if (n == 0) { return true; }
    return is_odd(n - 1);
}
#[mutual_tail_call]
fn is_odd(Int n) -> bool {
    if (n == 0) { return false; }
    return is_even(n - 1);
}
println(is_even(10));
println(is_odd(7));
"#);
        crate::feature_attrs::reset();
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "true");
    }

    #[test]
    fn even_odd_large_no_stack_overflow() {
        // Hold the test lock so reset() in other tests can't race the registry.
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        // Deep mutual recursion — without TCO this would blow the stack.
        let r = run(r#"
#[mutual_tail_call]
fn is_even(Int n) -> bool {
    if (n == 0) { return true; }
    return is_odd(n - 1);
}
#[mutual_tail_call]
fn is_odd(Int n) -> bool {
    if (n == 0) { return false; }
    return is_even(n - 1);
}
println(is_even(100000));
"#);
        crate::feature_attrs::reset();
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[test]
    fn mutual_tco_three_way() {
        // Hold the test lock so reset() in other tests can't race the registry.
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        // Three-way mutual recursion: a → b → c → a.
        let r = run(r#"
#[mutual_tail_call]
fn count_a(Int n) -> Int {
    if (n == 0) { return 0; }
    return count_b(n - 1);
}
#[mutual_tail_call]
fn count_b(Int n) -> Int {
    if (n == 0) { return 0; }
    return count_c(n - 1);
}
#[mutual_tail_call]
fn count_c(Int n) -> Int {
    if (n == 0) { return 0; }
    return count_a(n - 1);
}
println(count_a(99));
"#);
        crate::feature_attrs::reset();
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    #[test]
    fn missing_partner_annotation_is_error() {
        // Only one side of the mutual recursion has the attribute — should error.
        // run_program skips typechecking, so call check() directly like tail_calls tests do.
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = r#"
#[mutual_tail_call]
fn f(Int n) -> Int {
    if (n == 0) { return 0; }
    return n - 1;
}
"#;
        let prog = parse_program(src);
        let result = check(&prog, "<test>");
        crate::feature_attrs::reset();
        assert!(
            result.is_err(),
            "expected error for lone #[mutual_tail_call]"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("mutual_tail_call"),
            "error should mention mutual_tail_call: {}",
            msg
        );
    }
}
