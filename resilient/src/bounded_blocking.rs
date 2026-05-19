//! Ralph-Loop Uniqueness #18 — bounded-blocking budget for soft-real-time fns.
//!
//! Real-time systems classify functions as "non-blocking", "bounded
//! blocking", or "unbounded". POSIX has tagged functions as MT-safe /
//! AS-safe in documentation only. Rust async distinguishes "may yield"
//! but not "blocks for at most N ticks." No mainstream language enforces
//! a static blocking-call cap.
//!
//! Resilient enforces a budget by name suffix: any function ending in
//! `_bound1`, `_bound2`, `_bound4`, or `_bound8` may contain at most
//! that many calls to known blocking primitives in its transitive call
//! graph (within this translation unit). Blocking primitives are
//! `wait`, `sleep`, `recv`, `lock`, `acquire`, `block_on`, and any free
//! fn ending `_blocking`. Going over warns.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::visit;
use std::collections::{HashMap, HashSet, VecDeque};

const BLOCKING_PRIMS: &[&str] = &[
    "wait", "sleep", "recv", "lock", "acquire", "block_on", "park",
];

const SUFFIXES: &[(&str, usize)] = &[
    ("_bound1", 1),
    ("_bound2", 2),
    ("_bound4", 4),
    ("_bound8", 8),
];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    // RES-1218: fast-reject. The transitive-blocking analysis below
    // only fires for functions whose name ends in one of the
    // `_bound{N}` suffixes — the per-function `visit(body, ...)`
    // walk that builds `callees` + `blocking_calls` is dead work for
    // every other program. The overwhelming majority of `cargo test`
    // inputs declare zero `_bound{N}` functions, so a cheap O(N)
    // suffix scan over the top-level statement list short-circuits
    // the entire AST walk in that case. Mirrors RES-1211's
    // `isr_call_graph::check` fast-reject (which used the same shape
    // for `is_isr_name`).
    let has_bound_suffix = stmts.iter().any(|s| {
        if let Node::Function { name, .. } = &s.node {
            SUFFIXES.iter().any(|(suffix, _)| name.ends_with(*suffix))
        } else {
            false
        }
    });
    if !has_bound_suffix {
        return Ok(());
    }
    // RES-1511: borrow each top-level fn name as `&str` from the
    // AST into the outer `callees` / `blocking_calls` HashMaps and
    // the `decls` Vec. The inner `cs: Vec<String>` keeps owned
    // Strings because `uniqueness_walk::visit` uses a HRTB closure
    // that can't bind the AST lifetime (same limitation as
    // RES-1509 for `isr_call_graph::collect_callees`). Mirror of
    // RES-1495 / RES-1500 / RES-1503 / RES-1507 / RES-1509.
    // RES-1746: pre-size the three call-graph collections to stmts.len()
    // (upper bound). Same shape as RES-1742 / RES-1744 for the
    // sibling call-graph passes.
    let mut callees: HashMap<&str, Vec<String>> = HashMap::with_capacity(stmts.len());
    let mut blocking_calls: HashMap<&str, usize> = HashMap::with_capacity(stmts.len());
    let mut decls: Vec<&str> = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        if let Node::Function { name, body, .. } = &stmt.node {
            decls.push(name.as_str());
            // RES-1964: pre-size to 8 — typical fn bodies have 1-10
            // call sites. Skips the 0→4 first grow.
            let mut cs = Vec::with_capacity(8);
            let mut bn = 0;
            visit(body, &mut |n| {
                if let Node::CallExpression { function, .. } = n {
                    if let Node::Identifier { name: fname, .. } = function.as_ref() {
                        cs.push(fname.clone());
                        if BLOCKING_PRIMS.contains(&fname.as_str()) || fname.ends_with("_blocking")
                        {
                            bn += 1;
                        }
                    }
                }
            });
            callees.insert(name.as_str(), cs);
            blocking_calls.insert(name.as_str(), bn);
        }
    }
    for &d in &decls {
        let Some(budget) = SUFFIXES
            .iter()
            .find(|(s, _)| d.ends_with(*s))
            .map(|(_, n)| *n)
        else {
            continue;
        };
        let total = transitive_blocking(d, &callees, &blocking_calls);
        if total > budget {
            eprintln!(
                "warning: '{d}' declares blocking budget {budget} (by name suffix) \
                 but transitive blocking-call count is {total} — soft-real-time deadline at risk"
            );
        }
    }
    Ok(())
}

// RES-1471: borrow callee names as `&'a str` instead of cloning each
// one into the `seen` set and the BFS queue. All three parameters
// share lifetime `'a` so the borrows from `callees` and `start` can
// flow through `q` and `seen` without per-iteration `String::clone`.
fn transitive_blocking<'a>(
    start: &'a str,
    callees: &'a HashMap<&'a str, Vec<String>>,
    bn: &HashMap<&str, usize>,
) -> usize {
    let mut total = 0;
    // RES-1964: pre-size the BFS visited set and queue to
    // `callees.len()` — exact upper bound (each fn enqueued at most
    // once). Saves the 0→4→… doubling chain on every transitive_blocking
    // root walk.
    let mut seen: HashSet<&'a str> = HashSet::with_capacity(callees.len());
    let mut q: VecDeque<&'a str> = VecDeque::with_capacity(callees.len());
    q.push_back(start);
    while let Some(f) = q.pop_front() {
        if !seen.insert(f) {
            continue;
        }
        total += bn.get(f).copied().unwrap_or(0);
        if let Some(cs) = callees.get(f) {
            for c in cs {
                q.push_back(c.as_str());
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn empty_program_returns_ok() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn program_without_trigger_returns_ok() {
        let src = "fn f(int x) -> int { return x + 1; }\nf(5);\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
