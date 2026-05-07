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
    let mut callees: HashMap<String, Vec<String>> = HashMap::new();
    let mut blocking_calls: HashMap<String, usize> = HashMap::new();
    let mut decls: Vec<String> = Vec::new();
    for stmt in stmts {
        if let Node::Function { name, body, .. } = &stmt.node {
            decls.push(name.clone());
            let mut cs = Vec::new();
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
            callees.insert(name.clone(), cs);
            blocking_calls.insert(name.clone(), bn);
        }
    }
    for d in &decls {
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

fn transitive_blocking(
    start: &str,
    callees: &HashMap<String, Vec<String>>,
    bn: &HashMap<String, usize>,
) -> usize {
    let mut total = 0;
    let mut seen = HashSet::new();
    let mut q = VecDeque::new();
    q.push_back(start.to_string());
    while let Some(f) = q.pop_front() {
        if !seen.insert(f.clone()) {
            continue;
        }
        total += bn.get(&f).copied().unwrap_or(0);
        if let Some(cs) = callees.get(&f) {
            for c in cs {
                q.push_back(c.clone());
            }
        }
    }
    total
}
