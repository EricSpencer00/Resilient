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
    // RES-2230: borrow lock names and fn names from the AST throughout
    // the pair-collection + reporting flow. The previous shape paid:
    //   * one `lk.clone()` per lock/unlock call site (acts.push)
    //   * one `(prior.clone(), lk.clone())` per ordered pair recorded
    //   * one `fn_name.to_string()` per pair recorded
    //   * one `(b.clone(), a.clone())` per reverse-key lookup
    // None of those allocations outlives the program AST. The
    // hashmap-of-tuples-of-borrows lookup works via stdlib's
    // `Borrow<(&str, &str)>` on `(String, String)` is NOT supported,
    // but `HashMap<(&'a str, &'a str), _>` lookups with `(&'a str,
    // &'a str)` keys are direct.
    let mut pair_sites: HashMap<(&str, &str), Vec<&str>> = HashMap::new();
    for stmt in stmts {
        if let Node::Function { name, body, .. } = &stmt.node {
            collect_pairs(name.as_str(), body, &mut pair_sites);
        }
    }
    // RES-1524: borrow lock-name pairs into the dedup set instead
    // of cloning. The `pair_sites` map already owns the strings;
    // `reported` only checks "have I warned on this canonical
    // pair before". Same pattern as RES-1495 / RES-1500 / RES-1520
    // applied to a tuple key.
    let mut reported: HashSet<(&str, &str)> = HashSet::new();
    for (&(a, b), fns_ab) in &pair_sites {
        if let Some(fns_ba) = pair_sites.get(&(b, a)) {
            let canon: (&str, &str) = if a < b { (a, b) } else { (b, a) };
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
    Ok(())
}

fn collect_pairs<'a>(
    fn_name: &'a str,
    body: &'a Node,
    pairs: &mut HashMap<(&'a str, &'a str), Vec<&'a str>>,
) {
    // RES-1752: pre-size — a fn body typically has a small handful
    // of lock/unlock calls. 8 covers the common case without
    // doubling growth from 0.
    // RES-2230: held / acts now hold `&'a str` borrows from the AST.
    // `uniqueness_walk::visit<'a>` (RES-1603) propagates the AST
    // lifetime into the closure, so `lk.as_str()` is captured directly.
    let mut held: Vec<&'a str> = Vec::with_capacity(8);
    let mut acts: Vec<(bool, &'a str)> = Vec::with_capacity(8); // (is_lock, name)
    visit(body, &mut |n: &'a Node| {
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
                        acts.push((is_lock, lk.as_str()));
                    }
                }
            }
        }
    });
    for (is_lock, lk) in acts {
        if is_lock {
            for &prior in &held {
                if prior != lk {
                    pairs.entry((prior, lk)).or_default().push(fn_name);
                }
            }
            held.push(lk);
        } else {
            held.retain(|h| *h != lk);
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
