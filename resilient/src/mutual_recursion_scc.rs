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
        let Node::Program(stmts) = program else {
            return CallGraph {
                graph: HashMap::new(),
            };
        };

        // RES-1760: pre-size to stmts.len() — `build_graph_for_node`
        // walks top-level statements and inserts one entry per
        // function definition. Upper bound is stmts.len(). Same
        // pattern as the call-graph pre-size series.
        let mut graph = HashMap::with_capacity(stmts.len());
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

        // RES-1501: pre-size the per-pass `visited` set and the
        // `finish_stack` to `self.graph.len()`. Both grow exactly to
        // that bound (every node is visited and pushed exactly once).
        // The previous shape used `HashSet::new()` + `Vec::new()`,
        // triggering rehashes/reallocs at every default-bucket
        // boundary. Mirrors RES-1497 (transpose pre-size).
        let n = self.graph.len();

        // RES-1514: borrow each node name as `&str` from
        // `self.graph` / `transposed` rather than cloning into the
        // `visited` set, the `finish_stack`, and each per-SCC `Vec`.
        // The previous shape called `node.to_string()` four times per
        // DFS visit (visited.insert, finish_stack.push, second-pass
        // visited.insert, scc.push) — pure overhead since the source
        // strings already live in `self.graph` / `transposed`. The
        // owned `Vec<Vec<String>>` allocation only happens once at
        // the result-conversion step, matching what the return type
        // requires.

        // Step 1: DFS on original graph to get finish times (stack order).
        let mut visited: HashSet<&str> = HashSet::with_capacity(n);
        let mut finish_stack: Vec<&str> = Vec::with_capacity(n);
        for node in self.graph.keys() {
            if !visited.contains(node.as_str()) {
                dfs_finish_order(node.as_str(), &self.graph, &mut visited, &mut finish_stack);
            }
        }

        // Step 2: DFS on transposed graph in reverse finish order.
        let transposed = self.transpose();
        let mut visited: HashSet<&str> = HashSet::with_capacity(n);
        let mut sccs: Vec<Vec<&str>> = Vec::new();
        for node in finish_stack.into_iter().rev() {
            if !visited.contains(node) {
                let mut scc: Vec<&str> = Vec::new();
                dfs_collect(node, &transposed, &mut visited, &mut scc);
                sccs.push(scc);
            }
        }

        sccs.into_iter()
            .map(|scc| scc.into_iter().map(str::to_string).collect())
            .collect()
    }

    /// Find mutual recursion cycles (non-trivial SCCs of size > 1).
    /// Returns a list of cycles, where each cycle is a list of function names forming the cycle.
    pub fn find_mutual_recursion_cycles(&self) -> Vec<Vec<String>> {
        let sccs = self.find_sccs();
        sccs.into_iter().filter(|scc| scc.len() > 1).collect()
    }

    /// Return the transposed graph (edges reversed).
    ///
    /// RES-2086: keys and edge endpoints borrow `&str` from `self.graph`'s
    /// owned `String`s. The previous shape cloned each source name once for
    /// its self-entry plus once per outgoing edge (used as the destination's
    /// inserted endpoint), and each destination name once per incoming edge
    /// — `V + 2E` `String` allocations per Kosaraju call. Borrowing through
    /// `self` keeps the same hash-equality semantics (via `&str: Borrow<str>`)
    /// while dropping every allocation in this pass.
    fn transpose(&self) -> HashMap<&str, HashSet<&str>> {
        let mut transposed: HashMap<&str, HashSet<&str>> = HashMap::with_capacity(self.graph.len());

        for (src, dests) in &self.graph {
            transposed.entry(src.as_str()).or_default();
            for dest in dests {
                transposed
                    .entry(dest.as_str())
                    .or_default()
                    .insert(src.as_str());
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
            // RES-1465: use `entry().or_default()` instead of
            // `contains_key + insert`. The previous shape did two
            // hashed lookups + cloned `fn_name` twice (once for
            // contains_key's borrow check, once for insert's owned
            // key). `entry(key)` consumes the owned key once and
            // returns the slot whether present or absent.
            graph.entry(name.clone()).or_default();

            // Recursively walk the function body to find calls.
            // Pass `name` as `&str` so `collect_called_functions`'s
            // hash lookup doesn't need a fresh allocation.
            collect_called_functions(body, name, graph);
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
                // RES-1465: `current_fn` was already inserted into
                // `graph` by `build_graph_for_node`, so `get_mut`
                // always succeeds. The previous shape called
                // `entry(current_fn.to_string())` per call expression
                // — allocating a fresh `String` to look up a key that
                // was already in the map. `get_mut(&str)` does a
                // single hashed lookup with zero allocations.
                //
                // The `if let Some(...)` guard is defensive in case
                // `collect_called_functions` is ever entered with a
                // `current_fn` that wasn't pre-inserted; today the
                // only caller is `build_graph_for_node` which always
                // inserts first.
                if let Some(set) = graph.get_mut(current_fn) {
                    set.insert(name.clone());
                }
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
fn dfs_finish_order<'a>(
    node: &'a str,
    graph: &'a HashMap<String, HashSet<String>>,
    visited: &mut HashSet<&'a str>,
    finish_stack: &mut Vec<&'a str>,
) {
    visited.insert(node);

    if let Some(neighbors) = graph.get(node) {
        for neighbor in neighbors {
            if !visited.contains(neighbor.as_str()) {
                dfs_finish_order(neighbor.as_str(), graph, visited, finish_stack);
            }
        }
    }

    finish_stack.push(node);
}

/// DFS to collect SCC nodes (second DFS pass of Kosaraju on transposed graph).
///
/// RES-2086: `transposed` now holds borrowed `&'a str` keys and endpoints
/// (see `CallGraph::transpose`), so iteration can yield `&'a str` directly
/// without re-allocating per visit. The `'a` lifetime ties every borrowed
/// name back to `CallGraph::graph`, which outlives the whole SCC pass.
fn dfs_collect<'a>(
    node: &'a str,
    transposed: &HashMap<&'a str, HashSet<&'a str>>,
    visited: &mut HashSet<&'a str>,
    scc: &mut Vec<&'a str>,
) {
    visited.insert(node);
    scc.push(node);

    if let Some(neighbors) = transposed.get(node) {
        for &neighbor in neighbors {
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
