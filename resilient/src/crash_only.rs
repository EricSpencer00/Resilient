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
    // RES-1224: fast-reject. The diagnostic only fires for functions
    // whose name starts with `crash_`, so if none exists the
    // `HashSet<String>` of every top-level function name (N string
    // allocations plus bucket overhead) is pure waste. Programs
    // without `crash_*` functions — basically everything in
    // `examples/` and the test suite — get to skip the allocation
    // entirely. Same shape as RES-1211 / RES-1214 / RES-1217 /
    // RES-1218 / RES-1222.
    let has_crash = stmts
        .iter()
        .any(|s| matches!(&s.node, Node::Function { name, .. } if name.starts_with("crash_")));
    if !has_crash {
        return Ok(());
    }
    // RES-1520: borrow each top-level fn name as `&str` from the
    // AST into the lookup set. The contains check below uses
    // `&str` (via `Borrow<str>`), so the cloned `String` keys were
    // pure overhead. Same pattern as RES-1495 / RES-1500 etc.
    //
    // RES-1521: also reuse a single `needed` buffer for the
    // `recover_<suffix>` lookup key across iterations, instead of
    // `format!`-allocating a fresh `String` per `crash_*` function.
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
    let mut needed = String::new();
    for n in &names {
        if let Some(suffix) = n.strip_prefix("crash_") {
            needed.clear();
            needed.push_str("recover_");
            needed.push_str(suffix);
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
