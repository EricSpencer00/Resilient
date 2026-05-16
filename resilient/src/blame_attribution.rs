//! Feature 7/50 — Blame Attribution.
//!
//! When a `requires` clause is violated at runtime, the standard
//! diagnostic identifies the *callee* whose precondition wasn't
//! met. That's only half the story — the bug is usually at the
//! *caller*, who passed bad arguments.
//!
//! Blame Attribution maintains a static call graph at typecheck time
//! and exposes a `blame_chain(callee, depth)` API that walks backward
//! through the call graph to identify the root caller responsible for
//! a bad argument.
//!
//! Example: `main(int n) → process(int y) → validate(int x) requires x > 0`
//! If `n = -1`, `callers_of("validate")` names `process`, but
//! `blame_chain("validate", 3)` returns `["process", "main"]` — the
//! full ancestry pointing to the original source of the bad value.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::RwLock;

#[derive(Debug, Clone, Default)]
pub struct BlameMap {
    /// Key: callee fn name. Value: list of (caller_name, arg_index) pairs.
    pub edges: HashMap<String, Vec<(String, usize)>>,
}

impl BlameMap {
    /// Returns all immediate callers of `callee`.
    pub fn callers_of(&self, callee: &str) -> Vec<(String, usize)> {
        self.edges.get(callee).cloned().unwrap_or_default()
    }

    /// Returns the blame chain from `callee` backward through the call graph,
    /// up to `max_depth` hops. Returns callers in order from closest to
    /// farthest (BFS order). Each entry is (function_name, arg_index).
    ///
    /// Example: for `main → process → validate`, calling
    /// `blame_chain("validate", 3)` returns `[("process", 0), ("main", 0)]`.
    pub fn blame_chain(&self, callee: &str, max_depth: usize) -> Vec<(String, usize)> {
        let mut result = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(callee.to_string());

        // BFS queue: (fn_name, arg_idx_that_brought_us_here, depth)
        let mut queue: VecDeque<(String, usize, usize)> = VecDeque::new();
        if let Some(callers) = self.edges.get(callee) {
            for (caller, idx) in callers {
                if !visited.contains(caller) {
                    queue.push_back((caller.clone(), *idx, 1));
                }
            }
        }

        while let Some((node, arg_idx, depth)) = queue.pop_front() {
            if visited.contains(&node) {
                continue;
            }
            visited.insert(node.clone());
            result.push((node.clone(), arg_idx));

            if depth < max_depth {
                if let Some(callers) = self.edges.get(&node) {
                    for (caller, idx) in callers {
                        if !visited.contains(caller) {
                            queue.push_back((caller.clone(), *idx, depth + 1));
                        }
                    }
                }
            }
        }
        result
    }

    /// Format a human-readable blame chain for diagnostic output.
    /// Returns a string like `"main → process → validate"` where
    /// `validate` is the callee whose precondition failed.
    pub fn format_chain(&self, callee: &str, max_depth: usize) -> String {
        let chain = self.blame_chain(callee, max_depth);
        if chain.is_empty() {
            return callee.to_string();
        }
        let mut parts: Vec<&str> = chain.iter().map(|(n, _)| n.as_str()).collect();
        parts.reverse(); // root first
        parts.push(callee);
        parts.join(" → ")
    }
}

static BLAME_MAP: RwLock<Option<BlameMap>> = RwLock::new(None);

pub fn build(program: &Node) -> BlameMap {
    let mut map = BlameMap::default();
    let Node::Program(stmts) = program else {
        return map;
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            walk(body, name, &mut map);
        }
    }
    map
}

fn walk(node: &Node, caller: &str, map: &mut BlameMap) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name: callee, .. } = function.as_ref() {
                let entry = map.edges.entry(callee.clone()).or_default();
                for (idx, _) in arguments.iter().enumerate() {
                    entry.push((caller.to_string(), idx));
                }
            }
            for a in arguments {
                walk(a, caller, map);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk(s, caller, map);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk(condition, caller, map);
            walk(consequence, caller, map);
            if let Some(e) = alternative {
                walk(e, caller, map);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk(condition, caller, map);
            walk(body, caller, map);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk(iterable, caller, map);
            walk(body, caller, map);
        }
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            walk(value, caller, map)
        }
        Node::ReturnStatement { value: Some(e), .. } => walk(e, caller, map),
        Node::ExpressionStatement { expr, .. } => walk(expr, caller, map),
        Node::InfixExpression { left, right, .. } => {
            walk(left, caller, map);
            walk(right, caller, map);
        }
        Node::PrefixExpression { right, .. } => walk(right, caller, map),
        _ => {}
    }
}

pub fn install(map: BlameMap) {
    if let Ok(mut g) = BLAME_MAP.write() {
        *g = Some(map);
    }
}

/// Returns direct callers of `callee` from the installed map.
pub fn callers_of(callee: &str) -> Vec<(String, usize)> {
    BLAME_MAP
        .read()
        .ok()
        .and_then(|g| g.as_ref()?.edges.get(callee).cloned())
        .unwrap_or_default()
}

/// Returns the transitive blame chain for `callee` up to `max_depth` hops.
/// Callers are returned in BFS order (closest first). Empty when no callers.
pub fn blame_chain(callee: &str, max_depth: usize) -> Vec<(String, usize)> {
    BLAME_MAP
        .read()
        .ok()
        .and_then(|g| Some(g.as_ref()?.blame_chain(callee, max_depth)))
        .unwrap_or_default()
}

/// Format a human-readable blame chain for diagnostic output.
pub fn format_blame_chain(callee: &str, max_depth: usize) -> String {
    BLAME_MAP
        .read()
        .ok()
        .and_then(|g| Some(g.as_ref()?.format_chain(callee, max_depth)))
        .unwrap_or_else(|| callee.to_string())
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1291: fast-reject when no calls exist.
    let has_call =
        crate::uniqueness_walk::any_node(program, |n| matches!(n, Node::CallExpression { .. }));
    if !has_call {
        install(BlameMap::default());
        return Ok(());
    }
    let map = build(program);

    // At compile time, for every function with `requires` clauses, emit a
    // diagnostic when the blame chain reveals a root caller. This surfaces
    // likely precondition violations before the program runs.
    let Node::Program(stmts) = program else {
        install(map);
        return Ok(());
    };
    for s in stmts {
        if let Node::Function {
            name,
            requires,
            parameters,
            ..
        } = &s.node
        {
            if requires.is_empty() {
                continue;
            }
            let callers = map.callers_of(name);
            if callers.is_empty() {
                continue;
            }
            // Find the root callers via the transitive chain.
            let chain = map.blame_chain(name, 4);
            let root_callers: Vec<&str> = if chain.is_empty() {
                callers.iter().map(|(n, _)| n.as_str()).collect()
            } else {
                // Last entries in the chain are deepest; collect unique roots
                // in deepest-first order. `chain.len()` is the exact upper
                // bound on unique callers (each entry contributes at most
                // one name); the dedup against `roots.contains` only ever
                // shrinks the count.
                let mut roots: Vec<&str> = Vec::with_capacity(chain.len());
                for (caller, _) in chain.iter().rev() {
                    if !roots.contains(&caller.as_str()) {
                        roots.push(caller.as_str());
                    }
                }
                roots
            };
            let param_names: Vec<&str> = parameters.iter().map(|(_, n)| n.as_str()).collect();
            let chain_str = map.format_chain(name, 4);
            eprintln!(
                "blame: `{}` has `requires` on [{}]; root caller(s) responsible: [{}] (chain: {})",
                name,
                param_names.join(", "),
                root_callers.join(", "),
                chain_str
            );
        }
    }

    install(map);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn caller_is_attributed() {
        let src = r#"
            fn add(int a, int b) -> int requires b != 0 { return a + b; }
            fn main(int dummy) { let x = add(1, 2); return 0; }
        "#;
        let (prog, _) = parse(src);
        let map = build(&prog);
        let edges = map.edges.get("add").expect("add should have a caller");
        assert!(edges.iter().any(|(c, _)| c == "main"));
    }

    #[test]
    fn install_and_lookup_works() {
        let src = r#"
            fn helper(int x) { return x; }
            fn caller(int dummy) { let r = helper(42); return r; }
        "#;
        let (prog, _) = parse(src);
        let _ = check(&prog, "test");
        let callers = callers_of("helper");
        assert!(!callers.is_empty());
    }

    #[test]
    fn no_calls_no_blame() {
        let src = r#"fn solo(int x) { return x; }"#;
        let (prog, _) = parse(src);
        let map = build(&prog);
        assert!(!map.edges.contains_key("solo"));
    }

    #[test]
    fn blame_chain_two_hops() {
        // main → process → validate
        let src = r#"
            fn validate(int x) requires x > 0 { return x; }
            fn process(int y) { validate(y); }
            fn main(int n) { process(n); }
        "#;
        let (prog, _) = parse(src);
        let map = build(&prog);

        // Direct caller of validate is process
        let direct = map.callers_of("validate");
        assert!(direct.iter().any(|(c, _)| c == "process"));

        // Transitive chain should reach main
        let chain = map.blame_chain("validate", 3);
        let names: Vec<&str> = chain.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            names.contains(&"process"),
            "chain must include process; got: {names:?}"
        );
        assert!(
            names.contains(&"main"),
            "chain must include root caller main; got: {names:?}"
        );
    }

    #[test]
    fn format_chain_includes_callee() {
        let src = r#"
            fn validate(int x) requires x > 0 { return x; }
            fn caller(int n) { validate(n); }
        "#;
        let (prog, _) = parse(src);
        let map = build(&prog);
        let formatted = map.format_chain("validate", 2);
        assert!(
            formatted.contains("validate"),
            "chain must include callee; got: {formatted}"
        );
        assert!(
            formatted.contains("caller"),
            "chain must include caller; got: {formatted}"
        );
    }

    #[test]
    fn blame_chain_stops_at_max_depth() {
        // a → b → c → d → e: depth 2 should not reach e from a
        let src = r#"
            fn e(int x) { return x; }
            fn d(int x) { e(x); }
            fn c(int x) { d(x); }
            fn b(int x) { c(x); }
            fn a(int x) { b(x); }
        "#;
        let (prog, _) = parse(src);
        let map = build(&prog);
        let chain = map.blame_chain("e", 2);
        let names: Vec<&str> = chain.iter().map(|(n, _)| n.as_str()).collect();
        // At depth 2: e ← d ← c; should include d and c but not b
        assert!(
            !names.contains(&"a"),
            "depth-limited chain must not reach 'a'"
        );
        assert!(
            !names.contains(&"b"),
            "depth-limited chain must not reach 'b'"
        );
    }
}
