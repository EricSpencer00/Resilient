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
    // AST into the `callees` HashMap and `roots` Vec.
    //
    // RES-2062: also borrow inner call-site names. The earlier
    // comment cited "HRTB closure can't bind outer 'a" — but
    // `uniqueness_walk::visit<'a>` is a regular lifetime parameter,
    // not HRTB (same misread fixed by RES-2060 for isr_call_graph).
    // The closure can carry `'a` and the inserted &str borrows are
    // valid for the AST lifetime, which spans the whole check call
    // including the DFS in `reaches_self`.
    // RES-1742: pre-size the call-graph map to stmts.len() (upper
    // bound — every top-level statement could be a Function). The
    // per-fn callees HashSet starts empty; 8 fits a typical body.
    let mut callees: HashMap<&str, HashSet<&str>> = HashMap::with_capacity(stmts.len());
    // RES-1962: pre-size to 4 — typical non-reentrant fn count is low
    // (1-5 fns matching NR_PREFIXES / NR_SUFFIXES per program).
    let mut roots: Vec<&str> = Vec::with_capacity(4);
    for stmt in stmts {
        if let Node::Function { name, body, .. } = &stmt.node {
            let mut cs: HashSet<&str> = HashSet::with_capacity(8);
            visit(body, &mut |n| {
                if let Node::CallExpression { function, .. } = n
                    && let Node::Identifier { name, .. } = function.as_ref()
                {
                    cs.insert(name.as_str());
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

fn reaches_self<'a>(callees: &'a HashMap<&'a str, HashSet<&'a str>>, start: &str) -> bool {
    // RES-1962: pre-size the DFS visited set to `callees.len()` —
    // exact upper bound, each callee is visited at most once.
    // RES-2062: callees' inner set is now `&str`, so the seed can
    // copy through directly and the inner loop pushes the borrowed
    // name with no `.as_str()` round-trip.
    let mut seen: HashSet<&'a str> = HashSet::with_capacity(callees.len());
    let mut stack: Vec<&'a str> = callees
        .get(start)
        .map(|cs| cs.iter().copied().collect())
        .unwrap_or_default();
    while let Some(c) = stack.pop() {
        if c == start {
            return true;
        }
        if !seen.insert(c) {
            continue;
        }
        if let Some(cs) = callees.get(c) {
            for &cc in cs {
                stack.push(cc);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn no_nonreentrant_function_skips_check() {
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
    fn nonreentrant_with_no_recursion_returns_ok() {
        // V1 only emits warnings — always returns Ok.
        let src = "fn nonreentrant_handler(int x) -> int { return x + 1; }\n";
        let (prog, _) = parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "non-recursive nonreentrant function must not error in V1"
        );
    }
}
