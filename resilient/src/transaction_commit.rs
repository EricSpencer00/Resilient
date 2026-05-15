//! Ralph-Loop Uniqueness #4 — transaction commit-or-rollback on every path.
//!
//! Database libraries (Diesel, SQLAlchemy, Hibernate) leave commit/rollback
//! discipline to the programmer. Forgetting to commit is one of the most
//! common production bugs in transactional code; the `defer rb := tx.Rollback()`
//! / `try-with-resources` patterns help, but no language *requires* the call.
//!
//! Resilient encodes the contract:
//!
//!   - Any function with a parameter typed `Transaction` (or `Tx`,
//!     `&Transaction`, `&mut Transaction`) must, on every successful return
//!     path, contain at least one call to `commit(<tx>)` /
//!     `rollback(<tx>)` / `<tx>.commit()` / `<tx>.rollback()` /
//!     `<tx>.abort()` somewhere in the body.
//!   - Functions that bind a `Transaction` parameter and never close it
//!     emit a warning. (The "every-path" formulation is intentionally
//!     conservative; we currently require at least one closing call. CFG
//!     refinement is a follow-up.)

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{any_node, for_each_function};

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
    // RES-1218: fast-reject — see watchdog_feed for the same pattern.
    // Skip the closure dispatch + per-fn allocation for programs
    // that declare no `Transaction`/`Tx`-typed parameter.
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
        let unclosed: Vec<&str> = txs.into_iter().filter(|t| !is_closed(body, t)).collect();
        if !unclosed.is_empty() {
            eprintln!(
                "warning: function '{fname}' takes Transaction parameter(s) [{}] \
                 but never calls .commit()/.rollback()/.abort() — transaction may leak",
                unclosed.join(", ")
            );
        }
    });
    Ok(())
}

fn is_closed(body: &Node, tx: &str) -> bool {
    any_node(body, |n| match n {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
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
        _ => false,
    })
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
    fn tx_without_commit_returns_ok_v1() {
        // V1 emits a warning but always returns Ok.
        let src = "fn save(Transaction tx) { let x = 1; }\n";
        let (prog, _) = parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "V1 checker only warns, never returns Err"
        );
    }
}
