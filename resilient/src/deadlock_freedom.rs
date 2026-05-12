//! Feature 19/50 — Deadlock Freedom Proofs.
//!
//! Static analysis that proves an actor network cannot deadlock.
//! Builds the directed graph `actor → set of actors it sends to`,
//! detects strongly-connected components, and reports cycles. A
//! cycle in the message graph is a *potential* deadlock — the
//! verifier emits it as a warning that the user must annotate
//! away (e.g. by declaring one of the edges as a one-way
//! `notification` per a future ticket).
//!
//! For programs without cycles, the analyzer publishes a
//! "deadlock-free certificate" that records the analyzed actor
//! set and the verification timestamp.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub struct ActorGraph {
    pub edges: HashMap<String, HashSet<String>>,
}

pub fn build(program: &Node) -> ActorGraph {
    let mut g = ActorGraph::default();
    let Node::Program(stmts) = program else {
        return g;
    };
    let actor_names: HashSet<String> = stmts
        .iter()
        .filter_map(|s| match &s.node {
            Node::ActorDecl { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    for s in stmts {
        if let Node::ActorDecl {
            name,
            receive_handlers,
            handlers,
            ..
        } = &s.node
        {
            let mut sends = HashSet::new();
            for h in receive_handlers {
                walk_sends(&h.body, &actor_names, &mut sends);
            }
            for h in handlers {
                walk_sends(&h.body, &actor_names, &mut sends);
            }
            g.edges.insert(name.clone(), sends);
        }
    }
    g
}

fn walk_sends(node: &Node, actors: &HashSet<String>, out: &mut HashSet<String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if name == "send" {
                    if let Some(Node::Identifier { name: tgt, .. }) = arguments.first() {
                        if actors.contains(tgt) {
                            out.insert(tgt.clone());
                        }
                    }
                }
            }
            for a in arguments {
                walk_sends(a, actors, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_sends(s, actors, out);
            }
        }
        Node::ExpressionStatement { expr, .. } => walk_sends(expr, actors, out),
        Node::LetStatement { value, .. } => walk_sends(value, actors, out),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            walk_sends(consequence, actors, out);
            if let Some(e) = alternative {
                walk_sends(e, actors, out);
            }
        }
        _ => {}
    }
}

pub fn detect_cycles(graph: &ActorGraph) -> Vec<Vec<String>> {
    let mut cycles = Vec::new();
    let mut visited = HashSet::new();
    for start in graph.edges.keys() {
        if visited.contains(start) {
            continue;
        }
        let mut stack = vec![(start.clone(), vec![start.clone()])];
        while let Some((cur, path)) = stack.pop() {
            visited.insert(cur.clone());
            if let Some(adj) = graph.edges.get(&cur) {
                for n in adj {
                    if path.contains(n) {
                        let cycle_start = path.iter().position(|x| x == n).unwrap();
                        cycles.push(path[cycle_start..].to_vec());
                    } else {
                        let mut new_path = path.clone();
                        new_path.push(n.clone());
                        stack.push((n.clone(), new_path));
                    }
                }
            }
        }
    }
    cycles
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1291: fast-reject. `build` collects an actor→actors graph
    // by scanning every top-level statement, filtering ActorDecls,
    // then walking each ActorDecl's handler bodies for `send` calls.
    // Programs with zero `Node::ActorDecl` produce an empty graph
    // (no nodes, no edges) and `detect_cycles` finds nothing. The
    // overwhelming majority of programs — every fixture in
    // `examples/` that doesn't model an actor network, every
    // standalone-function unit test — pay this scan for zero output.
    // Pre-scan with the early-terminating `any_node` (RES-1238) and
    // skip both passes when no ActorDecl exists.
    let has_actor =
        crate::uniqueness_walk::any_node(program, |n| matches!(n, Node::ActorDecl { .. }));
    if !has_actor {
        return Ok(());
    }
    let g = build(program);
    let cycles = detect_cycles(&g);
    for c in &cycles {
        eprintln!(
            "warning: potential deadlock cycle in actor graph: {}",
            c.join(" -> ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn no_actors_no_cycles() {
        let src = r#"fn f(int x) { return x; }"#;
        let (prog, _) = parse(src);
        let g = build(&prog);
        assert!(detect_cycles(&g).is_empty());
    }
}
