//! Ralph-Loop Uniqueness #14 — static rate-limit fan-in.
//!
//! Web frameworks rate-limit at runtime via middleware. Some embedded
//! schedulers (FreeRTOS, Zephyr) verify periodicity of tasks. No
//! language *statically* checks the call-site cardinality of a function
//! that should only be invoked from a small number of places.
//!
//! Resilient enforces a static-call-site bound by name suffix: any
//! function whose name ends with `_oncepertick` or `_singleshot` may be
//! called from at most one site in the entire program; functions
//! ending with `_few` may have at most three call sites. Anything more
//! is a warning.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::visit;
use std::collections::HashMap;

const ONCE_SUFFIXES: &[&str] = &["_oncepertick", "_singleshot"];
const FEW_SUFFIX: &str = "_few";

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    // RES-1228: fast-reject. The diagnostic only fires for functions
    // whose name ends in `_oncepertick` / `_singleshot` / `_few`.
    // Programs without any such suffix don't need the program-wide
    // call-site count — yet the existing code walks every function
    // body and populates a `HashMap<String, usize>` of every callee
    // first, then loops `decls` looking for the suffix. Pre-scan
    // top-level names for the suffix once up front; if none match,
    // return immediately and skip both the AST traversals and the
    // HashMap allocation. Same shape as RES-1211 / RES-1214 /
    // RES-1217 / RES-1218 / RES-1222 / RES-1224.
    let has_rate_limited = stmts.iter().any(|s| {
        matches!(&s.node, Node::Function { name, .. }
            if ONCE_SUFFIXES.iter().any(|suf| name.ends_with(*suf)) || name.ends_with(FEW_SUFFIX))
    });
    if !has_rate_limited {
        return Ok(());
    }
    // RES-1515: borrow the top-level fn names into `decls` as `&str`
    // instead of cloning. The `counts` HashMap still needs owned
    // String keys because `uniqueness_walk::visit` uses a HRTB
    // closure that can't bind the outer AST lifetime (same
    // limitation hit by RES-1509 / RES-1511). For the counts
    // map, apply the RES-1505 entry-clone gate: only allocate
    // when the callee is new, falling back to `get_mut` for repeat
    // hits. Mirrors RES-1495 / RES-1500 / RES-1503 / RES-1507 /
    // RES-1508 / RES-1509 / RES-1511.
    let mut decls: Vec<&str> = Vec::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for stmt in stmts {
        if let Node::Function { name, body, .. } = &stmt.node {
            decls.push(name.as_str());
            visit(body, &mut |n| {
                if let Node::CallExpression { function, .. } = n {
                    if let Node::Identifier { name: callee, .. } = function.as_ref() {
                        if let Some(c) = counts.get_mut(callee.as_str()) {
                            *c += 1;
                        } else {
                            counts.insert(callee.clone(), 1);
                        }
                    }
                }
            });
        }
    }
    for &d in &decls {
        let limit = if ONCE_SUFFIXES.iter().any(|s| d.ends_with(*s)) {
            Some(1)
        } else if d.ends_with(FEW_SUFFIX) {
            Some(3)
        } else {
            None
        };
        if let Some(max) = limit {
            let c = counts.get(d).copied().unwrap_or(0);
            if c > max {
                eprintln!(
                    "warning: '{d}' is rate-limited (suffix says max {max} call site(s)) \
                     but the program contains {c} call sites — exceeds static budget"
                );
            }
        }
    }
    Ok(())
}
