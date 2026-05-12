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
                current_mod = name.clone();
                g.deps.entry(current_mod.clone()).or_default();
            }
            Node::Use { path, .. } => {
                g.deps
                    .entry(current_mod.clone())
                    .or_default()
                    .insert(path.clone());
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
    let mut visited: HashSet<&str> = HashSet::new();
    for start in graph.deps.keys() {
        let mut stack: Vec<&str> = Vec::new();
        if let Some(cycle) = dfs(start.as_str(), graph, &mut stack, &mut visited) {
            return Some(cycle.into_iter().map(str::to_string).collect());
        }
    }
    None
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    // RES-1246: fast-reject. `build` walks every top-level statement
    // looking for `Node::ModuleDecl` or `Node::Use`. For every
    // program that declares neither — basically everything in
    // `examples/` and most of the test suite — the walk produces an
    // empty `ModuleGraph::deps` and `detect_cycle` returns None.
    // Skip both calls by pre-scanning the top-level statement list
    // for the only two variants that contribute to the graph.
    if let Node::Program(stmts) = program {
        let has_module_or_use = stmts
            .iter()
            .any(|s| matches!(&s.node, Node::ModuleDecl { .. } | Node::Use { .. }));
        if !has_module_or_use {
            return Ok(());
        }
    }
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
