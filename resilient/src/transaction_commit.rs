//! Ralph-Loop Uniqueness #4 — transaction commit-or-rollback on every path.
//!
//! Database libraries (Diesel, SQLAlchemy, Hibernate) leave commit/rollback
//! discipline to the programmer. Forgetting to commit is one of the most
//! common production bugs in transactional code.
//!
//! Resilient encodes the contract:
//!
//!   - Any function with a parameter typed `Transaction` (or `Tx`,
//!     `&Transaction`, `&mut Transaction`) must call commit/rollback/abort
//!     on EVERY exit path through the function body.
//!   - The analysis is CFG-aware:
//!       * A linear close call covers all subsequent returns.
//!       * An if/else where BOTH branches close is treated as "closed" after
//!         the if/else.
//!       * An if-without-else can only cover the fall-through path if the
//!         consequence never returns (i.e., falls through to the close call
//!         after the if).
//!       * A return before any close call on that path is a violation.
//!   - Violations are reported as warnings (eprintln), not errors, to preserve
//!     backwards compatibility with existing code that relies on runtime
//!     close discipline.

#![allow(
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::for_each_function;

const TX_TYPES: &[&str] = &[
    "Transaction",
    "Tx",
    "&Transaction",
    "&mut Transaction",
    "&Tx",
    "&mut Tx",
];
const CLOSE_METHODS: &[&str] = &["commit", "rollback", "abort", "finish", "close"];
const CLOSE_FREE_FNS: &[&str] = &["commit", "rollback", "abort_tx", "commit_tx"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1218: fast-reject — skip allocation for programs with no Transaction params.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    let has_tx = stmts.iter().any(|s| {
        matches!(&s.node, Node::Function { parameters, .. }
            if parameters.iter().any(|(ty, _)| TX_TYPES.contains(&ty.as_str())))
    });
    if !has_tx {
        return Ok(());
    }
    for_each_function(program, |fname, params, body| {
        let txs: Vec<&str> = params
            .iter()
            .filter(|(ty, _)| TX_TYPES.contains(&ty.as_str()))
            .map(|(_, n)| n.as_str())
            .collect();
        if txs.is_empty() {
            return;
        }
        for tx in txs {
            match all_paths_close(body, tx) {
                PathClose::AlwaysCloses => {}
                PathClose::NeverCloses => {
                    eprintln!(
                        "warning: function '{fname}' takes Transaction parameter `{tx}` \
                         but never calls .commit()/.rollback()/.abort() — transaction may leak"
                    );
                }
                PathClose::UnclosedExit => {
                    eprintln!(
                        "warning: function '{fname}' parameter `{tx}`: not every return path \
                         calls .commit()/.rollback()/.abort() — transaction may leak on \
                         some control-flow paths"
                    );
                }
            }
        }
    });
    Ok(())
}

/// Three-state outcome of the CFG close analysis for one Transaction parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PathClose {
    /// Every exit (return or fall-through) from this subtree is preceded by a close call.
    AlwaysCloses,
    /// No path through this subtree contains a close call at all.
    NeverCloses,
    /// At least one exit lacks a close call (but a close call exists somewhere).
    UnclosedExit,
}

/// Returns the close status for `node` as a complete subtree.
/// The caller receives a value that describes whether every exit is covered.
fn all_paths_close(node: &Node, tx: &str) -> PathClose {
    if is_close_stmt(node, tx) {
        return PathClose::AlwaysCloses;
    }

    match node {
        Node::Block { stmts, .. } => block_paths_close(stmts, tx),
        Node::IfStatement {
            consequence,
            alternative: Some(alt),
            ..
        } => {
            let c = all_paths_close(consequence, tx);
            let a = all_paths_close(alt, tx);
            match (c, a) {
                (PathClose::AlwaysCloses, PathClose::AlwaysCloses) => PathClose::AlwaysCloses,
                (PathClose::NeverCloses, PathClose::NeverCloses) => PathClose::NeverCloses,
                _ => PathClose::UnclosedExit,
            }
        }
        Node::IfStatement {
            consequence,
            alternative: None,
            ..
        } => {
            // The fall-through path doesn't close tx. If the consequence has
            // any returns without a prior close, that's an unclosed exit.
            if has_return(consequence)
                && all_paths_close(consequence, tx) != PathClose::AlwaysCloses
            {
                PathClose::UnclosedExit
            } else {
                // Consequence either doesn't return (falls through) or always closes.
                // The fall-through path from the outer block is still open.
                PathClose::NeverCloses
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            // Loop body may not execute; we only care about returns inside.
            if has_return(body) && all_paths_close(body, tx) != PathClose::AlwaysCloses {
                PathClose::UnclosedExit
            } else {
                PathClose::NeverCloses
            }
        }
        Node::TryCatch { body, handlers, .. } => {
            let mut branches = Vec::with_capacity(handlers.len() + 1);
            branches.push(block_paths_close(body, tx));
            branches.extend(
                handlers
                    .iter()
                    .map(|(_, stmts)| block_paths_close(stmts, tx)),
            );
            merge_branches(branches)
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            let scrutinee_status = all_paths_close(scrutinee, tx);
            if scrutinee_status == PathClose::UnclosedExit {
                return PathClose::UnclosedExit;
            }
            let arm_statuses = arms.iter().map(|(_, guard, body)| {
                let body_status = all_paths_close(body, tx);
                guard.as_ref().map_or(body_status, |guard| {
                    sequence_status(all_paths_close(guard, tx), body_status)
                })
            });
            sequence_status(scrutinee_status, merge_branches(arm_statuses))
        }
        Node::ExpressionStatement { expr, .. }
        | Node::LetStatement { value: expr, .. }
        | Node::Assignment { value: expr, .. } => all_paths_close(expr, tx),
        Node::ReturnStatement {
            value: Some(value), ..
        } => {
            if all_paths_close(value, tx) == PathClose::AlwaysCloses {
                PathClose::AlwaysCloses
            } else {
                PathClose::UnclosedExit
            }
        }
        Node::ReturnStatement { .. } => PathClose::UnclosedExit,
        _ => PathClose::NeverCloses,
    }
}

/// Merge alternative control-flow branches. A branch that returns before
/// closing is always a hard failure; a mixture of closed and fall-through
/// branches is not fully covered either.
fn merge_branches(branches: impl IntoIterator<Item = PathClose>) -> PathClose {
    let mut saw_branch = false;
    let mut all_closed = true;
    for status in branches {
        saw_branch = true;
        if status != PathClose::AlwaysCloses {
            all_closed = false;
        }
        if status == PathClose::UnclosedExit {
            return PathClose::UnclosedExit;
        }
    }
    if saw_branch && all_closed {
        PathClose::AlwaysCloses
    } else if saw_branch {
        PathClose::UnclosedExit
    } else {
        PathClose::NeverCloses
    }
}

/// Compose two sequential pieces of control flow. A close in the prefix
/// covers everything that follows; otherwise the suffix determines whether
/// the path is covered.
fn sequence_status(prefix: PathClose, suffix: PathClose) -> PathClose {
    match prefix {
        PathClose::AlwaysCloses => PathClose::AlwaysCloses,
        PathClose::UnclosedExit => PathClose::UnclosedExit,
        PathClose::NeverCloses => suffix,
    }
}

/// Analyse a sequential statement list.
///
/// Processes statements in order. Once a close call is seen, all subsequent
/// exits are covered. An early return before a close is a violation.
fn block_paths_close(stmts: &[Node], tx: &str) -> PathClose {
    let mut closed = false;
    for stmt in stmts {
        if closed {
            continue;
        }
        if is_close_stmt(stmt, tx) {
            closed = true;
            continue;
        }
        match stmt {
            Node::ReturnStatement { .. } => {
                return PathClose::UnclosedExit;
            }
            Node::IfStatement {
                consequence,
                alternative: Some(alt),
                ..
            } => {
                let c = all_paths_close(consequence, tx);
                let a = all_paths_close(alt, tx);
                // If either branch has an unclosed exit, propagate immediately.
                if c == PathClose::UnclosedExit || a == PathClose::UnclosedExit {
                    return PathClose::UnclosedExit;
                }
                if c == PathClose::AlwaysCloses && a == PathClose::AlwaysCloses {
                    closed = true;
                }
                // If one branch closes and the other doesn't, the fall-through
                // path may be unclosed — flag it only when subsequent returns exist.
            }
            Node::IfStatement {
                consequence,
                alternative: None,
                ..
            } => {
                if has_return(consequence)
                    && all_paths_close(consequence, tx) != PathClose::AlwaysCloses
                {
                    return PathClose::UnclosedExit;
                }
            }
            Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
                if has_return(body) && all_paths_close(body, tx) != PathClose::AlwaysCloses {
                    return PathClose::UnclosedExit;
                }
            }
            Node::TryCatch { body, handlers, .. } => {
                let mut all_closed = true;
                for stmts in std::iter::once(body).chain(handlers.iter().map(|(_, stmts)| stmts)) {
                    match block_paths_close(stmts, tx) {
                        PathClose::AlwaysCloses => {}
                        PathClose::NeverCloses => all_closed = false,
                        PathClose::UnclosedExit => return PathClose::UnclosedExit,
                    }
                }
                if all_closed {
                    closed = true;
                }
            }
            Node::Match {
                scrutinee, arms, ..
            } => match all_paths_close(scrutinee, tx) {
                PathClose::AlwaysCloses => closed = true,
                PathClose::UnclosedExit => return PathClose::UnclosedExit,
                PathClose::NeverCloses => {
                    let mut all_closed = true;
                    for (_, guard, arm_body) in arms {
                        let arm_status = all_paths_close(arm_body, tx);
                        let arm_status = guard.as_ref().map_or(arm_status, |guard| {
                            sequence_status(all_paths_close(guard, tx), arm_status)
                        });
                        match arm_status {
                            PathClose::AlwaysCloses => {}
                            PathClose::NeverCloses => all_closed = false,
                            PathClose::UnclosedExit => return PathClose::UnclosedExit,
                        }
                    }
                    if all_closed && !arms.is_empty() {
                        closed = true;
                    }
                }
            },
            Node::ExpressionStatement { expr, .. }
            | Node::LetStatement { value: expr, .. }
            | Node::Assignment { value: expr, .. } => match all_paths_close(expr, tx) {
                PathClose::AlwaysCloses => closed = true,
                PathClose::UnclosedExit => return PathClose::UnclosedExit,
                PathClose::NeverCloses => {}
            },
            Node::Block { stmts: inner, .. } => {
                let inner_result = block_paths_close(inner, tx);
                if inner_result == PathClose::UnclosedExit {
                    return PathClose::UnclosedExit;
                }
                if inner_result == PathClose::AlwaysCloses {
                    closed = true;
                }
            }
            _ => {}
        }
    }
    if closed {
        PathClose::AlwaysCloses
    } else {
        PathClose::NeverCloses
    }
}

/// Returns `true` if `node` is a direct close call for `tx`.
fn is_close_stmt(node: &Node, tx: &str) -> bool {
    match node {
        Node::ExpressionStatement { expr, .. } => is_close_expr(expr, tx),
        Node::LetStatement { value, .. } => is_close_expr(value, tx),
        _ => is_close_expr(node, tx),
    }
}

fn is_close_expr(node: &Node, tx: &str) -> bool {
    let Node::CallExpression {
        function,
        arguments,
        ..
    } = node
    else {
        return false;
    };
    if let Node::FieldAccess { target, field, .. } = function.as_ref() {
        if CLOSE_METHODS.contains(&field.as_str())
            && matches!(target.as_ref(), Node::Identifier { name, .. } if name == tx)
        {
            return true;
        }
    }
    if let Node::Identifier { name, .. } = function.as_ref() {
        if CLOSE_FREE_FNS.contains(&name.as_str())
            && arguments
                .iter()
                .any(|a| matches!(a, Node::Identifier { name, .. } if name == tx))
        {
            return true;
        }
    }
    false
}

/// Returns `true` if `node` contains any `ReturnStatement` in its subtree.
fn has_return(node: &Node) -> bool {
    match node {
        Node::ReturnStatement { .. } => true,
        Node::Block { stmts, .. } => stmts.iter().any(has_return),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => has_return(consequence) || alternative.as_ref().is_some_and(|a| has_return(a)),
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => has_return(body),
        Node::TryCatch { body, handlers, .. } => {
            body.iter().any(has_return)
                || handlers
                    .iter()
                    .any(|(_, stmts)| stmts.iter().any(has_return))
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            has_return(scrutinee)
                || arms.iter().any(|(_, guard, body)| {
                    guard.as_ref().is_some_and(has_return) || has_return(body)
                })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn no_tx_param_skips_check() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn empty_program_returns_ok() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn tx_with_commit_before_return_is_ok() {
        let src = "fn save(Transaction tx) { commit(tx); return; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn tx_without_commit_returns_ok_warning_only() {
        // Violations emit warnings but check() still returns Ok
        let src = "fn save(Transaction tx) { let x = 1; }\n";
        let (prog, _) = parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "transaction checker only warns, never returns Err"
        );
    }

    #[test]
    fn path_analysis_never_closes() {
        let src = "fn save(Transaction tx) { let x = 1; }\n";
        let (prog, _) = parse(src);
        let Node::Program(stmts) = &prog else {
            panic!()
        };
        let Node::Function { body, .. } = &stmts[0].node else {
            panic!()
        };
        assert_eq!(all_paths_close(body, "tx"), PathClose::NeverCloses);
    }

    #[test]
    fn path_analysis_always_closes_linear() {
        let src = "fn save(Transaction tx) { commit(tx); return; }\n";
        let (prog, _) = parse(src);
        let Node::Program(stmts) = &prog else {
            panic!()
        };
        let Node::Function { body, .. } = &stmts[0].node else {
            panic!()
        };
        assert_eq!(all_paths_close(body, "tx"), PathClose::AlwaysCloses);
    }

    #[test]
    fn path_analysis_unclosed_early_return() {
        let src = r#"
fn save(Transaction tx, bool condition) {
    if condition {
        return;
    }
    commit(tx);
}
"#;
        let (prog, _) = parse(src);
        let Node::Program(stmts) = &prog else {
            panic!()
        };
        let Node::Function { body, .. } = &stmts[0].node else {
            panic!()
        };
        assert_eq!(all_paths_close(body, "tx"), PathClose::UnclosedExit);
    }

    #[test]
    fn path_analysis_if_else_both_close() {
        let src = r#"
fn save(Transaction tx, bool ok) {
    if ok {
        commit(tx);
    } else {
        rollback(tx);
    }
}
"#;
        let (prog, _) = parse(src);
        let Node::Program(stmts) = &prog else {
            panic!()
        };
        let Node::Function { body, .. } = &stmts[0].node else {
            panic!()
        };
        assert_eq!(all_paths_close(body, "tx"), PathClose::AlwaysCloses);
    }

    #[test]
    fn path_analysis_if_else_one_branch_missing() {
        let src = r#"
fn save(Transaction tx, bool ok) {
    if ok {
        commit(tx);
        return;
    } else {
        return;
    }
}
"#;
        let (prog, _) = parse(src);
        let Node::Program(stmts) = &prog else {
            panic!()
        };
        let Node::Function { body, .. } = &stmts[0].node else {
            panic!()
        };
        assert_eq!(all_paths_close(body, "tx"), PathClose::UnclosedExit);
    }

    fn function_body(src: &str) -> Node {
        let (prog, _) = parse(src);
        let Node::Program(mut stmts) = prog else {
            panic!()
        };
        let stmt = stmts.remove(0);
        let Node::Function { body, .. } = stmt.node else {
            panic!()
        };
        *body
    }

    #[test]
    fn path_analysis_try_body_return_is_unclosed() {
        let body = function_body(
            r#"
fn save(Transaction tx) {
    try {
        return;
    } catch Timeout {
        commit(tx);
    }
}
"#,
        );
        assert_eq!(all_paths_close(&body, "tx"), PathClose::UnclosedExit);
    }

    #[test]
    fn path_analysis_catch_return_is_unclosed() {
        let body = function_body(
            r#"
fn save(Transaction tx) {
    try {
        commit(tx);
    } catch Timeout {
        return;
    }
}
"#,
        );
        assert_eq!(all_paths_close(&body, "tx"), PathClose::UnclosedExit);
    }

    #[test]
    fn path_analysis_try_and_catch_both_close() {
        let body = function_body(
            r#"
fn save(Transaction tx) {
    try {
        commit(tx);
    } catch Timeout {
        rollback(tx);
    }
}
"#,
        );
        assert_eq!(all_paths_close(&body, "tx"), PathClose::AlwaysCloses);
    }

    #[test]
    fn path_analysis_match_arm_return_is_unclosed() {
        let body = function_body(
            r#"
fn save(Transaction tx, int value) {
    match value {
        0 => { return; }
        _ => { commit(tx); }
    }
}
"#,
        );
        assert_eq!(all_paths_close(&body, "tx"), PathClose::UnclosedExit);
    }

    #[test]
    fn path_analysis_match_arms_both_close() {
        let body = function_body(
            r#"
fn save(Transaction tx, int value) {
    match value {
        0 => commit(tx),
        _ => rollback(tx),
    }
}
"#,
        );
        assert_eq!(all_paths_close(&body, "tx"), PathClose::AlwaysCloses);
    }
}
