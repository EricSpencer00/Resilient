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
    let mut pair_sites: HashMap<(String, String), Vec<String>> = HashMap::new();
    for stmt in stmts {
        if let Node::Function { name, body, .. } = &stmt.node {
            collect_pairs(name, body, &mut pair_sites);
        }
    }
    let mut reported: HashSet<(String, String)> = HashSet::new();
    for ((a, b), fns_ab) in &pair_sites {
        let key_ba = (b.clone(), a.clone());
        if let Some(fns_ba) = pair_sites.get(&key_ba) {
            let canon = if a < b {
                (a.clone(), b.clone())
            } else {
                (b.clone(), a.clone())
            };
            if !reported.insert(canon.clone()) {
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

fn collect_pairs(fn_name: &str, body: &Node, pairs: &mut HashMap<(String, String), Vec<String>>) {
    let mut held: Vec<String> = Vec::new();
    let mut acts: Vec<(bool, String)> = Vec::new(); // (is_lock, name)
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
                        .entry((prior.clone(), lk.clone()))
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
