//! RES-398: termination checking — recursive fns require an explicit
//! `// @decreases <metric>` clause or `// @may_diverge` escape hatch.
//!
//! Reddit critique
//! (https://www.reddit.com/r/VibeCodersNest/comments/1ssv8ih/) asks:
//! *can an invalid state even be expressed in your system?* Today,
//! unbounded recursion is one such expressible-but-invalid state.
//! Resilient targets safety-critical embedded systems where
//! unbounded recursion is a known hazard (stack overflow on
//! Cortex-M, no graceful recovery).
//!
//! The runtime depth cap (RES-267) catches it at runtime. That's
//! *filtering*, not structural enforcement. This pass closes the
//! gap on direct (self-call) recursion: every directly-recursive
//! function must declare *either* a decreasing metric or that
//! divergence is acceptable.
//!
//! # Surface syntax
//!
//! Annotation goes on the line *immediately above* the `fn` keyword:
//!
//! ```text
//! // @decreases n
//! fn fact(int n) requires n >= 0 {
//!     if n <= 1 { return 1; }
//!     return n * fact(n - 1);
//! }
//!
//! // @may_diverge
//! fn event_loop() {
//!     loop_forever();
//! }
//! ```
//!
//! ## Why comment-based and not a new keyword?
//!
//! Comment-based annotation keeps the surface change minimal — no
//! new tokens, no parser arms, no AST node. The same line-offset
//! convention is already used by `// resilient: allow LXXXX` and
//! `// source:` (RES-397). A future ticket can promote this to a
//! first-class clause once the design has stabilized.
//!
//! # Behavior
//!
//! - **Default: off.** Existing programs are not affected. The
//!   pass returns `Ok(())` immediately when strict mode is not
//!   enabled, so cross-compile, REPL, and `rz run` are untouched.
//! - **Opt-in via `--strict-termination`** (CLI). When set,
//!   unannotated direct recursion produces a typechecker error.
//!
//! # Out of scope (future tickets)
//!
//! - **Mutual recursion**: this pass only catches direct self-calls.
//!   Follow-up: SCC-based call-graph analysis.
//! - **Z3 verification of the `decreases` metric**: the syntactic
//!   check lands first; SMT proof of strict decrease is a separate
//!   ticket.
//! - **Loop termination**: `while`/`for` are out of scope here;
//!   loop invariants (RES-132a) and loop-bound checks live elsewhere.

use crate::Node;
use std::sync::atomic::{AtomicBool, Ordering};

/// RES-398: process-wide flag controlling whether the termination
/// check is enforced. Off by default — existing programs see no
/// change. Mirrors the `bounds_check::DENY_UNPROVEN_BOUNDS` pattern.
static STRICT_TERMINATION: AtomicBool = AtomicBool::new(false);

/// Enable `--strict-termination` mode. Called from `main.rs` CLI
/// parsing before `check_program_with_source` runs.
pub fn set_strict_termination(on: bool) {
    STRICT_TERMINATION.store(on, Ordering::Relaxed);
}

fn strict_termination() -> bool {
    STRICT_TERMINATION.load(Ordering::Relaxed)
}

/// RES-398: typechecker extension pass — for every directly-recursive
/// top-level fn, require either `// @decreases <metric>` or
/// `// @may_diverge` on the line above the `fn` keyword. No-op
/// when strict mode is off.
pub fn check(program: &Node, source_path: &str) -> Result<(), String> {
    if !strict_termination() {
        return Ok(());
    }
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    let source = std::fs::read_to_string(source_path).unwrap_or_default();
    let lines: Vec<&str> = source.lines().collect();

    for spanned in stmts {
        if let Node::Function {
            name, body, span, ..
        } = &spanned.node
        {
            if name.is_empty() {
                continue;
            }
            if !calls_self(body, name) {
                continue;
            }
            let fn_line = span.start.line; // 1-indexed
            if fn_line < 2 {
                return Err(format!(
                    "{}:{}:{}: error: function `{}` is recursive but has no termination annotation; \
                     expected `// @decreases <metric>` or `// @may_diverge` on the line above",
                    source_path, fn_line, span.start.column, name
                ));
            }
            let prev = lines.get(fn_line - 2).copied().unwrap_or("");
            if !has_termination_annotation(prev) {
                return Err(format!(
                    "{}:{}:{}: error: function `{}` is recursive but has no termination annotation; \
                     expected `// @decreases <metric>` or `// @may_diverge` on the line above",
                    source_path, fn_line, span.start.column, name
                ));
            }
        }
    }
    Ok(())
}

/// Returns true when `line` (a single source line, no newline) carries
/// a `// @decreases <metric>` or `// @may_diverge` annotation. Leading
/// whitespace is ignored; trailing text after the annotation keyword
/// is treated as the metric / comment payload and is not validated
/// here (a future Z3 ticket will check the metric strictly decreases).
fn has_termination_annotation(line: &str) -> bool {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("// @decreases") {
        // Require at least one non-whitespace char after the keyword.
        return !rest.trim().is_empty();
    }
    if let Some(rest) = trimmed.strip_prefix("// @may_diverge") {
        // `// @may_diverge` alone is sufficient; trailing comment OK.
        return rest.is_empty() || rest.starts_with(char::is_whitespace);
    }
    false
}

/// Walks the AST looking for a `CallExpression` whose callee is the
/// identifier `name`. Returns true on the first hit. This catches
/// direct recursion — the function calling itself by name. Mutual
/// recursion (a → b → a) is intentionally out of scope; an SCC pass
/// is a separate ticket.
fn calls_self(node: &Node, name: &str) -> bool {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name: callee, .. } = function.as_ref()
                && callee == name
            {
                return true;
            }
            if calls_self(function, name) {
                return true;
            }
            arguments.iter().any(|a| calls_self(a, name))
        }
        Node::Block { stmts, .. } => stmts.iter().any(|s| calls_self(s, name)),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            calls_self(condition, name)
                || calls_self(consequence, name)
                || alternative.as_ref().is_some_and(|a| calls_self(a, name))
        }
        Node::WhileStatement {
            condition, body, ..
        } => calls_self(condition, name) || calls_self(body, name),
        Node::ForInStatement { iterable, body, .. } => {
            calls_self(iterable, name) || calls_self(body, name)
        }
        Node::LiveBlock { body, .. } => calls_self(body, name),
        Node::ReturnStatement { value: Some(v), .. } => calls_self(v, name),
        Node::ExpressionStatement { expr, .. } => calls_self(expr, name),
        Node::LetStatement { value, .. } => calls_self(value, name),
        Node::InfixExpression { left, right, .. } => {
            calls_self(left, name) || calls_self(right, name)
        }
        Node::PrefixExpression { right, .. } => calls_self(right, name),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotation_decreases_with_metric_accepted() {
        assert!(has_termination_annotation("// @decreases n"));
        assert!(has_termination_annotation("    // @decreases n - 1"));
        assert!(has_termination_annotation("// @decreases (a, b)"));
    }

    #[test]
    fn annotation_decreases_without_metric_rejected() {
        assert!(!has_termination_annotation("// @decreases"));
        assert!(!has_termination_annotation("// @decreases   "));
    }

    #[test]
    fn annotation_may_diverge_accepted() {
        assert!(has_termination_annotation("// @may_diverge"));
        assert!(has_termination_annotation("    // @may_diverge"));
        assert!(has_termination_annotation(
            "// @may_diverge — event loop is intentionally non-terminating"
        ));
    }

    #[test]
    fn unrelated_comment_rejected() {
        assert!(!has_termination_annotation("// just a comment"));
        assert!(!has_termination_annotation("// source: rfc-1234"));
        assert!(!has_termination_annotation(""));
    }

    // Note: the `check` function is exercised end-to-end via the
    // golden examples in `examples/termination_*.rz`. Unit-testing
    // it here would require constructing a full `Node::Program`
    // with span info, which the integration tests already do.
}
