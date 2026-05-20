//! Ralph-Loop Uniqueness #6 — canonical lock-acquisition ordering.
//!
//! Deadlock-by-inverted-locking is the textbook concurrency bug. Languages
//! deal with it via:
//!   * runtime detection (Java's `jstack`, Go's race detector — runtime!)
//!   * ownership/partial orders enforced *outside* the language (Linux
//!     kernel's lockdep)
//! No production language statically requires that whenever two locks
//! `A` and `B` are co-acquired, every co-acquisition site uses the same
//! global order.
//!
//! Resilient detects this. We collect, per function, the *order in which
//! distinct locks are acquired*: `lock(A); lock(B)` registers order
//! (A, B). Across the whole program, if any two functions register
//! conflicting pair-orders (A,B) and (B,A), we warn — that's a textbook
//! deadlock waiting to happen.
//!
//! Lock acquisition is detected by call sites named `lock`, `acquire`,
//! `mutex_lock`, `lock_<X>` whose first argument is an Identifier (the
//! lock name). Releases (`unlock`, `release`) are tracked too so a
//! re-acquired-after-release lock doesn't falsely register an order.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::visit;
use std::collections::{HashMap, HashSet};

const LOCK_FNS: &[&str] = &["lock", "acquire", "mutex_lock"];
const UNLOCK_FNS: &[&str] = &["unlock", "release", "mutex_unlock"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    // RES-1275 / RES-1917: the typechecker gates this call behind
    // a combined check on `markers.call_idents` for LOCK_FNS/UNLOCK_FNS
    // names and `lock_`/`unlock_` prefixes. The previous `any_node`
    // pre-scan was redundant — removed.
    //
    // RES-2430: nested map shape — `prior -> current -> fn_names`.
    // The previous flat `HashMap<(String, String), Vec<String>>`
    // forced the inversion-check loop to allocate two transient
    // `String`s per pair-site entry just to construct the reversed
    // `(b, a)` key (stdlib has no `Borrow<(&str, &str)>` impl for
    // `(String, String)`). Nested lookups use the existing
    // `String: Borrow<str>` impl on each level with zero
    // allocations. Same shape as RES-2008 / RES-2010 / RES-2012 /
    // RES-2014 / RES-2184 / RES-2194 / RES-2418.
    let mut pair_sites: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
    for stmt in stmts {
        if let Node::Function { name, body, .. } = &stmt.node {
            collect_pairs(name, body, &mut pair_sites);
        }
    }
    // RES-1524: borrow lock-name pairs into the dedup set instead
    // of cloning. The `pair_sites` map already owns the strings;
    // `reported` only checks "have I warned on this canonical
    // pair before".
    let mut reported: HashSet<(&str, &str)> = HashSet::new();
    for (a, by_b) in &pair_sites {
        for (b, fns_ab) in by_b {
            if let Some(fns_ba) = pair_sites.get(b).and_then(|m| m.get(a)) {
                let canon: (&str, &str) = if a < b {
                    (a.as_str(), b.as_str())
                } else {
                    (b.as_str(), a.as_str())
                };
                if !reported.insert(canon) {
                    continue;
                }
                eprintln!(
                    "warning: lock-ordering inversion: '{a}' before '{b}' in [{}] vs \
                     '{b}' before '{a}' in [{}] — deadlock risk",
                    fns_ab.join(", "),
                    fns_ba.join(", ")
                );
            }
        }
    }
    Ok(())
}

fn collect_pairs(
    fn_name: &str,
    body: &Node,
    pairs: &mut HashMap<String, HashMap<String, Vec<String>>>,
) {
    // RES-1752: pre-size — a fn body typically has a small handful
    // of lock/unlock calls. 8 covers the common case without
    // doubling growth from 0.
    let mut held: Vec<String> = Vec::with_capacity(8);
    let mut acts: Vec<(bool, String)> = Vec::with_capacity(8); // (is_lock, name)
    visit(body, &mut |n| {
        if let Node::CallExpression {
            function,
            arguments,
            ..
        } = n
        {
            if let Node::Identifier { name, .. } = function.as_ref() {
                let is_lock = LOCK_FNS.contains(&name.as_str()) || name.starts_with("lock_");
                let is_unlock = UNLOCK_FNS.contains(&name.as_str()) || name.starts_with("unlock_");
                if (is_lock || is_unlock) && !arguments.is_empty() {
                    if let Node::Identifier { name: lk, .. } = &arguments[0] {
                        acts.push((is_lock, lk.clone()));
                    }
                }
            }
        }
    });
    for (is_lock, lk) in acts {
        if is_lock {
            for prior in &held {
                if prior != &lk {
                    pairs
                        .entry(prior.clone())
                        .or_default()
                        .entry(lk.clone())
                        .or_default()
                        .push(fn_name.to_string());
                }
            }
            held.push(lk);
        } else {
            held.retain(|h| h != &lk);
        }
    }
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
