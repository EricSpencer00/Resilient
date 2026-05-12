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
    let names: HashSet<String> = stmts
        .iter()
        .filter_map(|s| {
            if let Node::Function { name, .. } = &s.node {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();
    for n in &names {
        if let Some(suffix) = n.strip_prefix("crash_") {
            let needed = format!("recover_{suffix}");
            if !names.contains(&needed) {
                eprintln!(
                    "warning: crash-only contract: '{n}' has no matching \
                     recovery function '{needed}' — crash path is unrecoverable"
                );
            }
        }
    }
    Ok(())
}
