//! Feature 40/50 — Full Module System.
//!
//! Extends the existing `modules.rs` (textual splicing) with a real
//! module graph: visibility modifiers, re-exports, and circular-
//! dependency detection.
//!
//! This first slice ships:
//! * Visibility modifier registry (per-item `pub` / `pub(crate)` /
//!   private).
//! * Module dependency graph derived from `use` statements.
//! * Cycle detector for the dependency graph.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Crate,
    Private,
}

impl Visibility {
    pub fn from_str(s: &str) -> Self {
        match s.trim() {
            "pub" => Visibility::Public,
            "pub(crate)" => Visibility::Crate,
            _ => Visibility::Private,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ModuleGraph {
    pub deps: HashMap<String, HashSet<String>>,
}

pub fn build(program: &Node) -> ModuleGraph {
    let mut g = ModuleGraph::default();
    let Node::Program(stmts) = program else {
        return g;
    };
    let mut current_mod = "__root".to_string();
    for s in stmts {
        match &s.node {
            Node::ModuleDecl { name, .. } => {
                // RES-2236: hoist the `current_mod` entry-init into the
                // ModuleDecl arm and use `or_default` *with* the owned
                // String we already have in hand. The previous shape did
                // `entry(current_mod.clone())` here AND `entry(
                // current_mod.clone())` on every subsequent Use — the Use
                // path's clone was paid per use-statement even though the
                // entry was already in the map. Borrow-then-fallback
                // collapses the steady-state per-Use cost to a single
                // hash probe.
                current_mod = name.clone();
                g.deps.entry(current_mod.clone()).or_default();
            }
            Node::Use { path, .. } => {
                // RES-2236: hot path — the entry exists once the enclosing
                // ModuleDecl has been processed (or after the first Use
                // under `__root`). `get_mut` borrows the key as `&str` and
                // skips the per-call `current_mod.clone()`. Only the cold
                // first-Use-without-ModuleDecl branch pays for the owned
                // String allocation now.
                if let Some(deps) = g.deps.get_mut(current_mod.as_str()) {
                    deps.insert(path.clone());
                } else {
                    let mut set = HashSet::new();
                    set.insert(path.clone());
                    g.deps.insert(current_mod.clone(), set);
                }
            }
            _ => {}
        }
    }
    g
}

pub fn detect_cycle(graph: &ModuleGraph) -> Option<Vec<String>> {
    // RES-1517: borrow node names through the DFS instead of
    // allocating a fresh `String` per visit. The DFS visited the
    // same node up to twice (`visited.insert(node.clone())` +
    // `on_stack.push(node.clone())`) for every reachable module —
    // pure overhead since the source strings already live in
    // `graph.deps`. The owned `Vec<String>` result is built once
    // at the public boundary. Same pattern as RES-1471 / RES-1474 /
    // RES-1477 / RES-1514.
    fn dfs<'a>(
        node: &'a str,
        graph: &'a ModuleGraph,
        on_stack: &mut Vec<&'a str>,
        visited: &mut HashSet<&'a str>,
    ) -> Option<Vec<&'a str>> {
        if let Some(idx) = on_stack.iter().position(|x| *x == node) {
            return Some(on_stack[idx..].to_vec());
        }
        if visited.contains(node) {
            return None;
        }
        visited.insert(node);
        on_stack.push(node);
        if let Some(adj) = graph.deps.get(node) {
            for n in adj {
                if let Some(cycle) = dfs(n.as_str(), graph, on_stack, visited) {
                    return Some(cycle);
                }
            }
        }
        on_stack.pop();
        None
    }
    // RES-1786: pre-size both to graph.deps.len() — visited grows
    // exactly to the module count; stack peaks at module-graph depth
    // which is bounded by the same count.
    let mut visited: HashSet<&str> = HashSet::with_capacity(graph.deps.len());
    for start in graph.deps.keys() {
        let mut stack: Vec<&str> = Vec::with_capacity(graph.deps.len());
        if let Some(cycle) = dfs(start.as_str(), graph, &mut stack, &mut visited) {
            return Some(cycle.into_iter().map(str::to_string).collect());
        }
    }
    None
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    // RES-1246 / RES-2316: the typechecker's `<EXTENSION_PASSES>`
    // dispatch gates this call behind
    // `markers.has_module_decl || markers.has_use`, so the program
    // is guaranteed to contain at least one ModuleDecl or Use. The
    // previous internal `stmts.iter().any(...)` pre-scan walked the
    // full top-level statement list a second time for the same
    // signal Markers already computed. Mirrors RES-2292 through
    // RES-2314.
    let g = build(program);
    if let Some(cycle) = detect_cycle(&g) {
        return Err(format!(
            "{}:0:0: error: circular module dependency: {}",
            source_path,
            cycle.join(" -> ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_parses() {
        assert_eq!(Visibility::from_str("pub"), Visibility::Public);
        assert_eq!(Visibility::from_str("pub(crate)"), Visibility::Crate);
        assert_eq!(Visibility::from_str(""), Visibility::Private);
    }

    #[test]
    fn empty_graph_has_no_cycle() {
        let g = ModuleGraph::default();
        assert!(detect_cycle(&g).is_none());
    }
}
