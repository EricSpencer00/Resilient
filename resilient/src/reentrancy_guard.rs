//! Ralph-Loop Uniqueness #7 — transitive reentrancy guard.
//!
//! Smart-contract languages (Solidity) and embedded RTOSes both suffer
//! reentrancy bugs. Solidity ships an OpenZeppelin `ReentrancyGuard`
//! mixin — runtime-only. Rust's borrow-checker prevents some forms but
//! has no concept of "this function may not transitively call itself."
//! No language statically detects mutual reentrancy across the call
//! graph.
//!
//! Resilient flags any function whose name has prefix `nonreentrant_`,
//! or matches a `_critical` / `_atomic` suffix, as a reentrancy-banned
//! root. Then it walks the static call graph and warns if the root is
//! transitively reachable from itself (direct or mutual recursion).

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::visit;
use std::collections::{HashMap, HashSet};

const NR_PREFIXES: &[&str] = &["nonreentrant_", "exclusive_"];
const NR_SUFFIXES: &[&str] = &["_critical", "_atomic", "_oneshot"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    // RES-1214: fast-reject. The warning loop only fires for
    // functions in `roots` — i.e. those whose names match
    // `is_nonreentrant` (NR-flavoured prefix/suffix). Programs that
    // declare no such function get an empty roots Vec and the BFS
    // never runs, but the callee-map population still walks every
    // function body. Skip the whole pass when there's no possible
    // root; `is_nonreentrant` itself is just a pair of
    // `&[&str]::iter().any(...)` string-prefix/suffix checks, far
    // cheaper than the per-body `visit` recursion it replaces here.
    let has_nonreentrant = stmts
        .iter()
        .any(|s| matches!(&s.node, Node::Function { name, .. } if is_nonreentrant(name)));
    if !has_nonreentrant {
        return Ok(());
    }
    // RES-1519: borrow each top-level fn name as `&str` from the
    // AST into the `callees` HashMap and `roots` Vec. The inner
    // `cs: HashSet<String>` keeps owned Strings because
    // `uniqueness_walk::visit` uses a HRTB closure that can't bind
    // the outer AST lifetime — same limitation hit by
    // RES-1509 / RES-1511. Pair with `reaches_self` borrowing into
    // the graph for the DFS (mirror of RES-1471 / RES-1474 / RES-1477).
    let mut callees: HashMap<&str, HashSet<String>> = HashMap::new();
    let mut roots: Vec<&str> = Vec::new();
    for stmt in stmts {
        if let Node::Function { name, body, .. } = &stmt.node {
            let mut cs = HashSet::new();
            visit(body, &mut |n| {
                if let Node::CallExpression { function, .. } = n {
                    if let Node::Identifier { name, .. } = function.as_ref() {
                        cs.insert(name.clone());
                    }
                }
            });
            callees.insert(name.as_str(), cs);
            if is_nonreentrant(name) {
                roots.push(name.as_str());
            }
        }
    }
    for root in roots {
        if reaches_self(&callees, root) {
            eprintln!(
                "warning: function '{root}' is non-reentrant (by name) but is \
                 transitively reachable from itself in the call graph — \
                 reentrancy will violate the exclusivity contract"
            );
        }
    }
    Ok(())
}

fn is_nonreentrant(name: &str) -> bool {
    NR_PREFIXES.iter().any(|p| name.starts_with(*p))
        || NR_SUFFIXES.iter().any(|s| name.ends_with(*s))
}

fn reaches_self<'a>(callees: &'a HashMap<&'a str, HashSet<String>>, start: &str) -> bool {
    let mut seen: HashSet<&'a str> = HashSet::new();
    let mut stack: Vec<&'a str> = callees
        .get(start)
        .map(|cs| cs.iter().map(String::as_str).collect())
        .unwrap_or_default();
    while let Some(c) = stack.pop() {
        if c == start {
            return true;
        }
        if !seen.insert(c) {
            continue;
        }
        if let Some(cs) = callees.get(c) {
            for cc in cs {
                stack.push(cc.as_str());
            }
        }
    }
    false
}
