//! Ralph-Loop Uniqueness #23 — epoch-ordering across function name suffixes.
//!
//! Migrations and schema-versioning require operations to run in epoch
//! order: phase-1 must finish before phase-2 starts. Database migration
//! tools enforce this at deploy time; no language requires that within
//! a single source file, calls to `*_epoch1` precede calls to `*_epoch2`
//! in any function that touches both.
//!
//! Resilient enforces a *lexical-order-within-function* property: in any
//! function body, if we see two calls in textual order — one to
//! `*_epoch<N>` and one to `*_epoch<M>` — N must be ≤ M. Out-of-order
//! epoch invocations warn.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{for_each_function, visit};

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1274 / RES-1917: the typechecker gates this call behind
    // `markers.any_call_ident_containing(&["_epoch"])`, so the program
    // is guaranteed to contain at least one `_epoch`-containing call
    // identifier. The previous `any_node` pre-scan was redundant —
    // removed.
    for_each_function(program, |fname, _params, body| {
        // RES-2164: borrow the call name as `&str` from the body AST.
        // The sequence vector lives only inside this for_each_function
        // callback iteration and the consumer just `eprintln!`s the
        // name — the previous `name.clone()` per epoch-tagged call
        // was throwaway.
        let mut sequence: Vec<(&str, u32)> = Vec::new();
        visit(body, &mut |n| {
            if let Node::CallExpression { function, .. } = n {
                if let Node::Identifier { name, .. } = function.as_ref() {
                    if let Some(e) = epoch_of(name) {
                        sequence.push((name.as_str(), e));
                    }
                }
            }
        });
        for w in sequence.windows(2) {
            let (a, ea) = &w[0];
            let (b, eb) = &w[1];
            if ea > eb {
                eprintln!(
                    "warning: in '{fname}', epoch-ordered call '{a}' (epoch {ea}) \
                     precedes '{b}' (epoch {eb}) — epochs must be non-decreasing"
                );
            }
        }
    });
    Ok(())
}

fn epoch_of(name: &str) -> Option<u32> {
    // Match suffix _epoch<N> with N a non-negative integer.
    let idx = name.rfind("_epoch")?;
    let tail = &name[idx + "_epoch".len()..];
    tail.parse::<u32>().ok()
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
    fn program_without_epoch_calls_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn epoch_of_parses_suffix() {
        assert_eq!(epoch_of("migrate_epoch1"), Some(1));
        assert_eq!(epoch_of("migrate_epoch42"), Some(42));
        assert_eq!(epoch_of("no_epoch"), None);
    }
}
