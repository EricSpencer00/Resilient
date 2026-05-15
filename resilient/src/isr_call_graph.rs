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
    // RES-1211: fast-reject. The pass only emits diagnostics for ISR
    // functions and their transitive callees; if the program declares
    // no ISR, every later step is a no-op (`isr_roots` stays empty
    // and the BFS never enters its loop). Scan the top-level function
    // names first — that's O(N) over toplevel statements with a cheap
    // suffix/prefix string check — and skip the per-body
    // `collect_callees` walks, which would otherwise dominate this
    // pass on non-embedded programs (the overwhelming majority of
    // `examples/` and the test suite).
    let has_isr = stmts
        .iter()
        .any(|s| matches!(&s.node, Node::Function { name, .. } if is_isr_name(name)));
    if !has_isr {
        return Ok(());
    }
    // RES-1509: borrow each top-level fn name as `&str` from the
    // AST instead of cloning into the `callees` HashMap key and
    // `isr_roots` Vec. `collect_callees` inside the visit closure
    // still produces owned Strings (HRTB closure can't bind the
    // outer `'a`), but the *outer* map keys and root list don't
    // need ownership — same pattern as RES-1495 / RES-1500 etc.
    // RES-1744: pre-size the call-graph map to stmts.len() (upper
    // bound). Same shape as RES-1742 for reentrancy_guard.
    let mut callees: HashMap<&str, HashSet<String>> = HashMap::with_capacity(stmts.len());
    let mut isr_roots: Vec<&str> = Vec::new();
    for stmt in stmts {
        if let Node::Function { name, body, .. } = &stmt.node {
            callees.insert(name.as_str(), collect_callees(body));
            if is_isr_name(name) {
                isr_roots.push(name.as_str());
            }
        }
    }
    // RES-1474: borrow into `callees` / `isr_roots` for the BFS
    // instead of cloning each name into `seen` and `q`. The HashMap
    // and Vec both live for the duration of this block, so `&str`
    // borrows from them remain valid across the BFS. Mirror of
    // RES-1471's `bounded_blocking::transitive_blocking` refactor.
    for &root in &isr_roots {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut q: VecDeque<&str> = VecDeque::new();
        q.push_back(root);
        while let Some(fname) = q.pop_front() {
            if !seen.insert(fname) {
                continue;
            }
            if let Some(cs) = callees.get(fname) {
                for c in cs {
                    if is_isr_unsafe_call(c) {
                        eprintln!(
                            "warning: ISR '{root}' transitively calls ISR-hostile '{c}' \
                             via '{fname}' — interrupt context must not block or allocate"
                        );
                    }
                    q.push_back(c.as_str());
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
