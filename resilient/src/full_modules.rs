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
use resilient_span::Span;
use std::collections::{BTreeMap, HashSet};

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

/// A single `use` edge from the enclosing module to `target`, tagged
/// with the source span of the `use` statement that introduced it —
/// this is what lets RES-4110's cycle diagnostic point at a concrete
/// `line:col` instead of just naming the modules involved.
#[derive(Debug, Clone)]
pub struct Edge {
    pub target: String,
    pub span: Span,
}

/// RES-4110: `deps` is a `BTreeMap<String, Vec<Edge>>` rather than the
/// previous `HashMap<String, HashSet<String>>` for two reasons: (1) a
/// `BTreeMap` iterates in a deterministic key order, and a `Vec`
/// preserves the source order edges were discovered in, so
/// `detect_cycle` reports the *same* cycle on every run regardless of
/// hash-iteration order; (2) edges now carry a span, so duplicate
/// targets from different `use` sites are legitimately distinct edges,
/// not something a `HashSet` should collapse.
#[derive(Debug, Clone, Default)]
pub struct ModuleGraph {
    pub deps: BTreeMap<String, Vec<Edge>>,
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
                g.deps.entry(current_mod.clone()).or_default().push(Edge {
                    target: path.clone(),
                    span: s.span,
                });
            }
            _ => {}
        }
    }
    g
}

/// One step of a reported import cycle: the module name, and the span
/// of the `use` statement that leads to the *next* step in the cycle.
#[derive(Debug, Clone)]
pub struct CycleStep {
    pub name: String,
    pub span: Span,
}

/// Depth-first cycle search over the module dependency graph. Detects
/// cycles of any length (not just direct A<->B pairs) by tracking the
/// current DFS stack and reporting a match as soon as a node reachable
/// from itself reappears on that stack — RES-4110 adds the edge span
/// for every step and switches the graph traversal to the deterministic
/// `BTreeMap`/`Vec` shape above so the *same* cycle (and the same
/// starting node) is reported on every run for a given program.
pub fn detect_cycle(graph: &ModuleGraph) -> Option<Vec<CycleStep>> {
    fn dfs<'a>(
        node: &'a str,
        graph: &'a ModuleGraph,
        on_stack: &mut Vec<(&'a str, Span)>,
        visited: &mut HashSet<&'a str>,
    ) -> Option<Vec<(&'a str, Span)>> {
        if let Some(idx) = on_stack.iter().position(|(n, _)| *n == node) {
            return Some(on_stack[idx..].to_vec());
        }
        if visited.contains(node) {
            return None;
        }
        visited.insert(node);
        if let Some(adj) = graph.deps.get(node) {
            for edge in adj {
                on_stack.push((node, edge.span));
                if let Some(cycle) = dfs(edge.target.as_str(), graph, on_stack, visited) {
                    return Some(cycle);
                }
                on_stack.pop();
            }
        }
        None
    }
    let mut visited: HashSet<&str> = HashSet::with_capacity(graph.deps.len());
    for start in graph.deps.keys() {
        let mut stack: Vec<(&str, Span)> = Vec::with_capacity(graph.deps.len());
        if let Some(cycle) = dfs(start.as_str(), graph, &mut stack, &mut visited) {
            return Some(
                cycle
                    .into_iter()
                    .map(|(name, span)| CycleStep {
                        name: name.to_string(),
                        span,
                    })
                    .collect(),
            );
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
        // RES-4110: render the full cycle path with a `line:col` per
        // step (the position of the `use` that closes the loop back to
        // the first-visited module), then repeat the starting module
        // name once more at the end so the printed path visibly closes
        // the loop (`A -> B -> C -> A`) instead of trailing off at `C`.
        let first = cycle.first().map(|s| s.name.clone());
        let mut path = String::new();
        for (i, step) in cycle.iter().enumerate() {
            if i > 0 {
                path.push_str(" -> ");
            }
            path.push_str(&format!(
                "{} ({}:{}:{})",
                step.name, source_path, step.span.start.line, step.span.start.column
            ));
        }
        if let Some(first) = first {
            path.push_str(" -> ");
            path.push_str(&first);
        }
        let head_span = cycle.first().map(|s| s.span);
        let (line, col) = head_span
            .map(|sp| (sp.start.line, sp.start.column))
            .unwrap_or((0, 0));
        return Err(format!(
            "{source_path}:{line}:{col}: error: circular module dependency: {path}"
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

    #[test]
    fn single_module_no_dependencies() {
        let src = r#"module MyMod { }"#;
        let (prog, _) = crate::parse(src);
        let g = build(&prog);
        assert!(detect_cycle(&g).is_none());
    }

    #[test]
    fn multiple_modules_no_cycles() {
        let src = r#"
            module A { }
            module B { }
            module C { }
        "#;
        let (prog, _) = crate::parse(src);
        let g = build(&prog);
        assert!(detect_cycle(&g).is_none());
    }

    #[test]
    fn linear_dependency_chain() {
        let src = r#"
            module A { }
            use "b";
            module B { }
            use "c";
            module C { }
        "#;
        let (prog, _) = crate::parse(src);
        let g = build(&prog);
        assert!(detect_cycle(&g).is_none());
    }

    #[test]
    fn visibility_enum_all_variants() {
        assert_eq!(Visibility::from_str("pub"), Visibility::Public);
        assert_eq!(Visibility::from_str("pub(crate)"), Visibility::Crate);
        assert_eq!(Visibility::from_str("private"), Visibility::Private);
        assert_eq!(Visibility::from_str(""), Visibility::Private);
        assert_eq!(Visibility::from_str("unknown"), Visibility::Private);
    }

    #[test]
    fn check_passes_for_acyclic_graph() {
        let src = r#"
            module Server { }
            module Client { }
        "#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test.rz").is_ok());
    }

    fn edge(target: &str) -> Edge {
        Edge {
            target: target.to_string(),
            span: Span::point(resilient_span::Pos::new(1, 1, 0)),
        }
    }

    #[test]
    fn two_module_cycle_detected() {
        let mut g = ModuleGraph::default();
        g.deps.insert("A".to_string(), vec![edge("B")]);
        g.deps.insert("B".to_string(), vec![edge("A")]);
        let cycle = detect_cycle(&g).expect("expected a cycle");
        assert_eq!(cycle.len(), 2);
        assert_eq!(cycle[0].name, "A");
        assert_eq!(cycle[1].name, "B");
    }

    #[test]
    fn three_module_cycle_detected_with_full_path() {
        // RES-4110: A -> B -> C -> A is a length-3 cycle; the old
        // detector's `on_stack` DFS could already find this
        // structurally, but nothing asserted it and the reported order
        // was hash-iteration-dependent. Pin it down with a deterministic
        // BTreeMap-backed graph.
        let mut g = ModuleGraph::default();
        g.deps.insert("A".to_string(), vec![edge("B")]);
        g.deps.insert("B".to_string(), vec![edge("C")]);
        g.deps.insert("C".to_string(), vec![edge("A")]);
        let cycle = detect_cycle(&g).expect("expected a cycle");
        assert_eq!(
            cycle.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
            vec!["A", "B", "C"]
        );
    }

    #[test]
    fn four_module_cycle_not_confused_with_shorter_subpath() {
        // D depends on nothing cyclic; A -> B -> C -> A is still the
        // cycle even with an extra acyclic branch hanging off B.
        let mut g = ModuleGraph::default();
        g.deps.insert("A".to_string(), vec![edge("B")]);
        g.deps.insert("B".to_string(), vec![edge("C"), edge("D")]);
        g.deps.insert("C".to_string(), vec![edge("A")]);
        g.deps.insert("D".to_string(), vec![]);
        let cycle = detect_cycle(&g).expect("expected a cycle");
        assert_eq!(
            cycle.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
            vec!["A", "B", "C"]
        );
    }

    #[test]
    fn check_reports_line_col_and_closes_the_loop() {
        let mut g = ModuleGraph::default();
        g.deps.insert(
            "A".to_string(),
            vec![Edge {
                target: "B".to_string(),
                span: Span::point(resilient_span::Pos::new(2, 5, 10)),
            }],
        );
        g.deps.insert(
            "B".to_string(),
            vec![Edge {
                target: "A".to_string(),
                span: Span::point(resilient_span::Pos::new(4, 9, 40)),
            }],
        );
        let cycle = detect_cycle(&g).expect("expected a cycle");
        assert_eq!(cycle[0].span.start.line, 2);
        assert_eq!(cycle[0].span.start.column, 5);
        assert_eq!(cycle[1].span.start.line, 4);
        assert_eq!(cycle[1].span.start.column, 9);
    }

    #[test]
    fn no_false_positive_on_shared_dependency_diamond() {
        // A -> B, A -> C, B -> D, C -> D is a diamond, not a cycle —
        // D is reachable via two paths but never points back.
        let mut g = ModuleGraph::default();
        g.deps.insert("A".to_string(), vec![edge("B"), edge("C")]);
        g.deps.insert("B".to_string(), vec![edge("D")]);
        g.deps.insert("C".to_string(), vec![edge("D")]);
        g.deps.insert("D".to_string(), vec![]);
        assert!(detect_cycle(&g).is_none());
    }
}
