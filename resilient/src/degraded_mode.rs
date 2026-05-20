//! Ralph-Loop Uniqueness #20 — degraded-mode branch is mandatory after
//! a critical-assert failure.
//!
//! High-reliability systems define a *degraded mode* — a code path that
//! runs after a critical postcondition has been observed false but the
//! system must not crash. Aerospace standards (DO-178C) require it;
//! no language has a syntactic mandate that "after a critical assert,
//! the next statement on the failure branch must call into a degraded-
//! mode fn."
//!
//! Resilient enforces it pattern-wise: any `if !cond { ... }` whose
//! body's first statement is a call to a fn named with prefix
//! `assert_critical_` (or `panic` / `abort`) must NOT be the only
//! statement — a sibling statement is required, and at least one of the
//! statements in the same enclosing block following the assert must
//! call into a `degraded_` / `safe_mode_` / `recover_` fn. We warn
//! when a critical assert is used as a "shoot then disappear" pattern.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::for_each_function;

const CRITICAL_PREFIXES: &[&str] = &["assert_critical_", "abort_", "halt_"];
const RECOVERY_PREFIXES: &[&str] = &["degraded_", "safe_mode_", "recover_"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1252 / RES-2364: the typechecker gates this call behind
    // `markers.any_call_ident_with_prefix(CRITICAL_PREFIXES)`, so
    // the program is guaranteed to contain at least one
    // `CallExpression` whose callee has a CRITICAL prefix. The
    // previous internal `any_node` pre-scan was redundant — removed
    // along with the `any_node` import.
    for_each_function(program, |fname, _params, body| {
        scan_blocks(fname, body);
    });
    Ok(())
}

fn scan_blocks(fname: &str, node: &Node) {
    if let Node::Block { stmts, .. } = node {
        for (i, s) in stmts.iter().enumerate() {
            if calls_critical(s) {
                let after = &stmts[i + 1..];
                if !after.iter().any(calls_recovery_anywhere) {
                    eprintln!(
                        "warning: in '{fname}', a critical assert/abort was used \
                         without a sibling degraded_/safe_mode_/recover_ call \
                         in the same block — degraded-mode requirement"
                    );
                }
            }
        }
    }
    crate::uniqueness_walk::walk_children(node, &mut |c| scan_blocks(fname, c));
}

fn calls_critical(node: &Node) -> bool {
    crate::uniqueness_walk::any_node(node, |n| match n {
        Node::CallExpression { function, .. } => match function.as_ref() {
            Node::Identifier { name, .. } => CRITICAL_PREFIXES.iter().any(|p| name.starts_with(*p)),
            _ => false,
        },
        _ => false,
    })
}

fn calls_recovery_anywhere(node: &Node) -> bool {
    crate::uniqueness_walk::any_node(node, |n| match n {
        Node::CallExpression { function, .. } => match function.as_ref() {
            Node::Identifier { name, .. } => RECOVERY_PREFIXES.iter().any(|p| name.starts_with(*p)),
            _ => false,
        },
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_program_returns_ok() {
        let (prog, _) = crate::parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn program_without_critical_call_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn critical_prefixes_include_assert_critical() {
        assert!(
            CRITICAL_PREFIXES
                .iter()
                .any(|p| p.contains("assert_critical"))
        );
        assert!(RECOVERY_PREFIXES.iter().any(|p| p.contains("degraded_")));
    }
}
