//! RES-796: Mutual recursion termination check via SCC (Strongly Connected Component) analysis.
//!
//! This module implements Kosaraju's algorithm to detect cycles in the function call graph,
//! identifying mutual recursion patterns that the direct-recursion checker (RES-398) misses.
//!
//! A strongly-connected component (SCC) is a maximal set of vertices such that there is a
//! path from each vertex to every other vertex. An SCC of size > 1 in the call graph
//! indicates mutual recursion (cycles). An SCC of size 1 with a self-loop is direct recursion
//! (already handled by RES-398).

use crate::Node;
use std::collections::{HashMap, HashSet};

/// Represents a function call graph where edges are function calls.
#[derive(Debug, Clone)]
pub struct CallGraph {
    /// Map from function name to set of functions it calls.
    graph: HashMap<String, HashSet<String>>,
}

impl CallGraph {
    /// Build a call graph by walking all function definitions in the program.
    pub fn build(program: &Node) -> Self {
        let mut graph = HashMap::new();
        let Node::Program(stmts) = program else {
            return CallGraph { graph };
        };

        for stmt in stmts {
            build_graph_for_node(&stmt.node, &mut graph);
        }

        CallGraph { graph }
    }

    /// Check if a function calls itself (direct recursion).
    pub fn has_self_call(&self, fn_name: &str) -> bool {
        self.graph
            .get(fn_name)
            .map(|calls| calls.contains(fn_name))
            .unwrap_or(false)
    }

    /// Find all strongly-connected components using Kosaraju's algorithm.
    pub fn find_sccs(&self) -> Vec<Vec<String>> {
        if self.graph.is_empty() {
            return vec![];
        }

        // Step 1: DFS on original graph to get finish times (stack order).
        let mut visited = HashSet::new();
        let mut finish_stack = Vec::new();
        for node in self.graph.keys() {
            if !visited.contains(node) {
                dfs_finish_order(node, &self.graph, &mut visited, &mut finish_stack);
            }
        }

        // Step 2: DFS on transposed graph in reverse finish order.
        let transposed = self.transpose();
        let mut visited = HashSet::new();
        let mut sccs = Vec::new();
        for node in finish_stack.into_iter().rev() {
            if !visited.contains(&node) {
                let mut scc = Vec::new();
                dfs_collect(&node, &transposed, &mut visited, &mut scc);
                sccs.push(scc);
            }
        }

        sccs
    }

    /// Find mutual recursion cycles (non-trivial SCCs of size > 1).
    /// Returns a list of cycles, where each cycle is a list of function names forming the cycle.
    pub fn find_mutual_recursion_cycles(&self) -> Vec<Vec<String>> {
        let sccs = self.find_sccs();
        sccs.into_iter().filter(|scc| scc.len() > 1).collect()
    }

    /// Return the transposed graph (edges reversed).
    fn transpose(&self) -> HashMap<String, HashSet<String>> {
        let mut transposed: HashMap<String, HashSet<String>> = HashMap::new();

        for (src, dests) in &self.graph {
            transposed.entry(src.clone()).or_default();
            for dest in dests {
                transposed
                    .entry(dest.clone())
                    .or_default()
                    .insert(src.clone());
            }
        }

        transposed
    }
}

/// Walk a node and record all function calls made within it.
/// `current_fn` tracks which function we're currently analyzing.
fn build_graph_for_node(node: &Node, graph: &mut HashMap<String, HashSet<String>>) {
    match node {
        Node::Function { name, body, .. } => {
            let fn_name = name.clone();
            if !graph.contains_key(&fn_name) {
                graph.insert(fn_name.clone(), HashSet::new());
            }

            // Recursively walk the function body to find calls
            collect_called_functions(body, &fn_name, graph);
        }
        Node::Program(stmts) => {
            for stmt in stmts {
                build_graph_for_node(&stmt.node, graph);
            }
        }
        _ => {}
    }
}

/// Collect all functions called within a node, recording them in the graph
/// under the `current_fn` entry.
fn collect_called_functions(
    node: &Node,
    current_fn: &str,
    graph: &mut HashMap<String, HashSet<String>>,
) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            // Extract function name from identifier or field access
            if let Node::Identifier { name, .. } = function.as_ref() {
                graph
                    .entry(current_fn.to_string())
                    .or_default()
                    .insert(name.clone());
            }

            // Recurse into arguments
            for arg in arguments {
                collect_called_functions(arg, current_fn, graph);
            }
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                collect_called_functions(stmt, current_fn, graph);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            condition,
            ..
        } => {
            collect_called_functions(condition, current_fn, graph);
            collect_called_functions(consequence, current_fn, graph);
            if let Some(alt) = alternative {
                collect_called_functions(alt, current_fn, graph);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_called_functions(condition, current_fn, graph);
            collect_called_functions(body, current_fn, graph);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_called_functions(iterable, current_fn, graph);
            collect_called_functions(body, current_fn, graph);
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_called_functions(scrutinee, current_fn, graph);
            for (_, guard, body) in arms {
                if let Some(g) = guard {
                    collect_called_functions(g, current_fn, graph);
                }
                collect_called_functions(body, current_fn, graph);
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            collect_called_functions(expr, current_fn, graph);
        }
        Node::ReturnStatement {
            value: Some(val), ..
        } => {
            collect_called_functions(val, current_fn, graph);
        }
        Node::InfixExpression { left, right, .. } => {
            collect_called_functions(left, current_fn, graph);
            collect_called_functions(right, current_fn, graph);
        }
        Node::PrefixExpression { right, .. } => {
            collect_called_functions(right, current_fn, graph);
        }
        Node::StructLiteral { fields, .. } => {
            for (_, value) in fields {
                collect_called_functions(value, current_fn, graph);
            }
        }
        Node::ArrayLiteral { items, .. } => {
            for item in items {
                collect_called_functions(item, current_fn, graph);
            }
        }
        Node::FieldAccess { target, .. } => {
            collect_called_functions(target, current_fn, graph);
        }
        _ => {}
    }
}

/// DFS to establish finish order (first DFS pass of Kosaraju).
fn dfs_finish_order(
    node: &str,
    graph: &HashMap<String, HashSet<String>>,
    visited: &mut HashSet<String>,
    finish_stack: &mut Vec<String>,
) {
    visited.insert(node.to_string());

    if let Some(neighbors) = graph.get(node) {
        for neighbor in neighbors {
            if !visited.contains(neighbor) {
                dfs_finish_order(neighbor, graph, visited, finish_stack);
            }
        }
    }

    finish_stack.push(node.to_string());
}

/// DFS to collect SCC nodes (second DFS pass of Kosaraju on transposed graph).
fn dfs_collect(
    node: &str,
    transposed: &HashMap<String, HashSet<String>>,
    visited: &mut HashSet<String>,
    scc: &mut Vec<String>,
) {
    visited.insert(node.to_string());
    scc.push(node.to_string());

    if let Some(neighbors) = transposed.get(node) {
        for neighbor in neighbors {
            if !visited.contains(neighbor) {
                dfs_collect(neighbor, transposed, visited, scc);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn scc_detects_simple_mutual_recursion() {
        let src = "fn f(int n) { if (n > 0) { g(n - 1); } }
                   fn g(int n) { if (n > 0) { f(n + 1); } }";
        let (program, _) = parse(src);
        let graph = CallGraph::build(&program);
        let sccs = graph.find_sccs();

        // Should find exactly one SCC containing {f, g}
        assert_eq!(sccs.len(), 1);
        let scc = &sccs[0];
        assert_eq!(scc.len(), 2);
        assert!(scc.contains(&"f".to_string()));
        assert!(scc.contains(&"g".to_string()));
    }

    #[test]
    fn scc_detects_complex_cycle() {
        let src = "fn a(int n) { b(n); }
                   fn b(int n) { c(n); }
                   fn c(int n) { a(n); }";
        let (program, _) = parse(src);
        let graph = CallGraph::build(&program);
        let sccs = graph.find_sccs();

        // Should find one SCC with all three functions
        assert!(sccs.iter().any(|scc| scc.len() == 3));
    }

    #[test]
    fn scc_allows_direct_recursion() {
        let src = "fn f(int n) { if (n > 0) { f(n - 1); } }";
        let (program, _) = parse(src);
        let graph = CallGraph::build(&program);
        let sccs = graph.find_sccs();

        // Self-loop is size-1 SCC; RES-398 already handles it
        assert!(sccs.iter().any(|scc| scc.len() == 1));
    }

    #[test]
    fn scc_ignores_non_recursive_code() {
        let src = "fn f(int n) { return n + 1; }
                   fn g(int n) { return n * 2; }";
        let (program, _) = parse(src);
        let graph = CallGraph::build(&program);
        let sccs = graph.find_sccs();

        // Each function is its own SCC (no edges)
        let single_sccs: Vec<_> = sccs.iter().filter(|scc| scc.len() == 1).collect();
        assert_eq!(single_sccs.len(), 2);
    }
}
