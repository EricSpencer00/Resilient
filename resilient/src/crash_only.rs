//! Ralph-Loop Uniqueness #21 — crash-only modules.
//!
//! "Crash-only software" (Candea & Fox 2003) argues that the cleanest
//! shutdown is a crash + recovery. Erlang/OTP achieves this in practice
//! via supervisor restart, but no *language* makes the dual structural
//! requirement: if you have a `crash_*` function, you must also have a
//! `recover_*` function with a matching suffix.
//!
//! Resilient enforces the dual: for every function whose name starts
//! with `crash_`, there must be a sibling function named `recover_` +
//! the same suffix. Otherwise we warn that the crash path has no
//! recovery handler.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use std::collections::HashSet;

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    // RES-1224 / RES-2310: the typechecker's `<EXTENSION_PASSES>`
    // dispatch gates this call behind
    // `markers.any_fn_name_with_prefix(&["crash_"])`, so the program
    // is guaranteed to contain at least one `crash_`-prefixed
    // function. The previous internal `stmts.iter().any(...)`
    // pre-scan walked the full top-level statement list a second
    // time for the same signal Markers already computed. Mirrors
    // RES-2292 through RES-2308.
    // RES-1520: borrow each top-level fn name as `&str` from the
    // AST into the lookup set. The contains check below uses
    // `&str` (via `Borrow<str>`), so the cloned `String` keys were
    // pure overhead. Same pattern as RES-1495 / RES-1500 etc.
    let names: HashSet<&str> = stmts
        .iter()
        .filter_map(|s| {
            if let Node::Function { name, .. } = &s.node {
                Some(name.as_str())
            } else {
                None
            }
        })
        .collect();
    for n in &names {
        if let Some(suffix) = n.strip_prefix("crash_") {
            let needed = format!("recover_{suffix}");
            if !names.contains(needed.as_str()) {
                eprintln!(
                    "warning: crash-only contract: '{n}' has no matching \
                     recovery function '{needed}' — crash path is unrecoverable"
                );
            }
        }
    }
    Ok(())
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
    fn program_without_crash_fn_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn crash_with_matching_recover_returns_ok() {
        let src = "fn crash_network() { return 0; }\nfn recover_network() { return 0; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
