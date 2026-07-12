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
        // RES-2346: stream the previous epoch call instead of
        // collecting every match into `Vec<(String, u32)>`. The
        // pairwise check only ever consults the immediately
        // preceding call (the `windows(2)` loop), so a running
        // `Option<(&str, u32)>` produces the same diagnostics
        // without per-call `String::clone` and without the Vec
        // allocation/second pass. `visit<'a>(…, FnMut(&'a Node))`
        // (RES-1238) propagates the AST lifetime so `name.as_str()`
        // is valid across closure invocations.
        let mut prev: Option<(&str, u32)> = None;
        visit(body, &mut |n| {
            if let Node::CallExpression { function, .. } = n
                && let Node::Identifier { name, .. } = function.as_ref()
                && let Some(e) = epoch_of(name)
            {
                if let Some((a, ea)) = prev
                    && ea > e
                {
                    eprintln!(
                        "warning: in '{fname}', epoch-ordered call '{a}' (epoch {ea}) \
                         precedes '{name}' (epoch {e}) — epochs must be non-decreasing"
                    );
                }
                prev = Some((name.as_str(), e));
            }
        });
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

    #[test]
    fn ordered_epoch_calls_pass() {
        let src = r#"
            fn process() {
                migrate_epoch1();
                migrate_epoch2();
                migrate_epoch3();
            }
        "#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn same_epoch_repeated() {
        let src = r#"
            fn duplicate_calls() {
                do_epoch5();
                do_epoch5();
                do_epoch5();
            }
        "#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn mixed_epoch_and_non_epoch_calls() {
        let src = r#"
            fn mixed_flow() {
                setup_epoch1();
                regular_task();
                migrate_epoch2();
                cleanup();
                verify_epoch3();
            }
        "#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn zero_epoch_value() {
        let src = r#"
            fn start_epochs() {
                init_epoch0();
                main_epoch0();
                finalize_epoch1();
            }
        "#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn large_epoch_numbers() {
        let src = r#"
            fn long_sequence() {
                phase_epoch100();
                phase_epoch200();
                phase_epoch500();
                phase_epoch999();
            }
        "#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
