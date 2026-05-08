//! Ralph-Loop Uniqueness #5 — ISR-safety transitive call-graph check.
//!
//! On bare-metal systems an interrupt-service-routine must not allocate,
//! must not block, and must not call into anything that does either.
//! Ada/SPARK has `Pragma Synchronous` on subprograms but lacks a
//! transitive ISR-call-graph check. C and C++ rely on convention. Rust
//! has no language-level concept of "this function runs in interrupt
//! context."
//!
//! Resilient flags as ISR-context any function whose name is suffixed
//! `_isr` / `_irq` or prefixed `isr_` / `irq_`, then performs a
//! transitive call-graph walk and warns if any callee is a known
//! "ISR-hostile" primitive: `malloc`, `free`, `panic`, `lock`, `wait`,
//! `sleep`, `println`, `print`, `block_on`, `await`, or any function in
//! the program flagged `_blocks` / `_alloc`.
//!
//! The user gets a real defect class — "this ISR transitively allocates"
//! — at compile time, not at oscilloscope time.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::visit;
use std::collections::{HashMap, HashSet, VecDeque};

const ISR_NAME_HINTS: &[&str] = &["_isr", "_irq"];
const ISR_NAME_PREFIXES: &[&str] = &["isr_", "irq_"];
const UNSAFE_PRIMS: &[&str] = &[
    "malloc",
    "free",
    "panic",
    "lock",
    "wait",
    "sleep",
    "block_on",
    "println",
    "print",
    "spawn",
    "actor_send_blocking",
];
const UNSAFE_NAME_SUFFIXES: &[&str] = &["_blocks", "_alloc", "_blocking"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    let mut callees: HashMap<String, HashSet<String>> = HashMap::new();
    let mut isr_roots: Vec<String> = Vec::new();
    for stmt in stmts {
        if let Node::Function { name, body, .. } = &stmt.node {
            callees.insert(name.clone(), collect_callees(body));
            if is_isr_name(name) {
                isr_roots.push(name.clone());
            }
        }
    }
    for root in isr_roots {
        let mut seen = HashSet::new();
        let mut q = VecDeque::new();
        q.push_back(root.clone());
        while let Some(fname) = q.pop_front() {
            if !seen.insert(fname.clone()) {
                continue;
            }
            if let Some(cs) = callees.get(&fname) {
                for c in cs {
                    if is_isr_unsafe_call(c) {
                        eprintln!(
                            "warning: ISR '{root}' transitively calls ISR-hostile '{c}' \
                             via '{fname}' — interrupt context must not block or allocate"
                        );
                    }
                    q.push_back(c.clone());
                }
            }
        }
    }
    Ok(())
}

fn is_isr_name(name: &str) -> bool {
    ISR_NAME_HINTS.iter().any(|s| name.ends_with(*s))
        || ISR_NAME_PREFIXES.iter().any(|p| name.starts_with(*p))
}

fn is_isr_unsafe_call(name: &str) -> bool {
    UNSAFE_PRIMS.contains(&name) || UNSAFE_NAME_SUFFIXES.iter().any(|s| name.ends_with(*s))
}

fn collect_callees(body: &Node) -> HashSet<String> {
    let mut out = HashSet::new();
    visit(body, &mut |n| {
        if let Node::CallExpression { function, .. } = n {
            if let Node::Identifier { name, .. } = function.as_ref() {
                out.insert(name.clone());
            }
        }
    });
    out
}
