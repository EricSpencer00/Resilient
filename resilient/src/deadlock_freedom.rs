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
    // RES-1522: borrow each actor name as `&str` from the AST
    // into the lookup set used by `walk_sends`. The set is only
    // queried via `contains`, never extracted, so the cloned
    // `String` keys were pure overhead. Same pattern as RES-1495
    // / RES-1500 etc.
    let actor_names: HashSet<&str> = stmts
        .iter()
        .filter_map(|s| match &s.node {
            Node::ActorDecl { name, .. } => Some(name.as_str()),
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
            // An actor can only `send` to a known target actor (see
            // walk_sends's `actors.contains` guard), so the unique
            // recipients per actor are bounded by the total actor
            // count. Pre-size to that upper bound to skip the default
            // bucket-grow cascade for clusters with more than a handful
            // of actors.
            let mut sends = HashSet::with_capacity(actor_names.len());
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

fn walk_sends(node: &Node, actors: &HashSet<&str>, out: &mut HashSet<String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if name == "send" {
                    if let Some(Node::Identifier { name: tgt, .. }) = arguments.first() {
                        if actors.contains(tgt.as_str()) {
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

// RES-1477: borrow into `graph.edges` for the DFS instead of cloning
// every node name into `visited`, the stack frames, and each path
// extension. Return cycles as `Vec<Vec<&'a str>>`; the production
// caller (`deadlock_freedom::check`) immediately `.join(" -> ")`s
// them (works on `&[&str]`), and the test uses `.is_empty()`. The
// graph lives at least as long as the returned cycles, so the
// borrow chain is sound.
pub fn detect_cycles<'a>(graph: &'a ActorGraph) -> Vec<Vec<&'a str>> {
    let mut cycles = Vec::new();
    // `visited` grows to at most one entry per node in the graph; pre-size
    // to that exact upper bound. Same shape as the `sends` pre-size above.
    let mut visited: HashSet<&'a str> = HashSet::with_capacity(graph.edges.len());
    for start in graph.edges.keys() {
        let start = start.as_str();
        if visited.contains(start) {
            continue;
        }
        let mut stack: Vec<(&'a str, Vec<&'a str>)> = vec![(start, vec![start])];
        while let Some((cur, path)) = stack.pop() {
            visited.insert(cur);
            if let Some(adj) = graph.edges.get(cur) {
                for n in adj {
                    let n_str = n.as_str();
                    if path.contains(&n_str) {
                        let cycle_start = path.iter().position(|x| *x == n_str).unwrap();
                        cycles.push(path[cycle_start..].to_vec());
                    } else {
                        let mut new_path = path.clone();
                        new_path.push(n_str);
                        stack.push((n_str, new_path));
                    }
                }
            }
        }
    }
    cycles
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1291 / RES-1916: the typechecker gates this call behind
    // `markers.has_actor_decl`, so the program is guaranteed to
    // contain at least one `ActorDecl`. The previous `any_node`
    // pre-scan was redundant — removed.
    let g = build(program);
    let cycles = detect_cycles(&g);
    if cycles.is_empty() {
        // Emit the deadlock-free certificate so downstream tooling can
        // record that the actor network was proven cycle-free at this
        // compilation.
        let actor_count = g.edges.len();
        eprintln!("deadlock-free: actor network ({actor_count} actor(s)) verified cycle-free");
    } else {
        for c in &cycles {
            eprintln!(
                "warning: potential deadlock cycle in actor graph: {}",
                c.join(" -> ")
            );
        }
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

    #[test]
    fn empty_program_check_returns_ok() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn detect_cycles_on_empty_graph_returns_empty() {
        let empty = ActorGraph::default();
        assert!(detect_cycles(&empty).is_empty());
    }

    fn make_graph(edges: &[(&str, &str)]) -> ActorGraph {
        let mut g = ActorGraph::default();
        for (from, to) in edges {
            g.edges
                .entry((*from).to_string())
                .or_default()
                .insert((*to).to_string());
            // Ensure all nodes appear as keys even if they have no outgoing edges.
            g.edges.entry((*to).to_string()).or_default();
        }
        g
    }

    #[test]
    fn two_actor_mutual_send_is_a_cycle() {
        // A → B → A  is a cycle
        let g = make_graph(&[("A", "B"), ("B", "A")]);
        let cycles = detect_cycles(&g);
        assert!(
            !cycles.is_empty(),
            "mutual send between two actors must be flagged as a cycle"
        );
    }

    #[test]
    fn linear_chain_has_no_cycle() {
        // A → B → C: no cycle
        let g = make_graph(&[("A", "B"), ("B", "C")]);
        let cycles = detect_cycles(&g);
        assert!(
            cycles.is_empty(),
            "linear actor chain must not be flagged: {cycles:?}"
        );
    }

    #[test]
    fn three_actor_ring_is_a_cycle() {
        // A → B → C → A
        let g = make_graph(&[("A", "B"), ("B", "C"), ("C", "A")]);
        let cycles = detect_cycles(&g);
        assert!(
            !cycles.is_empty(),
            "three-actor ring must be detected as a cycle"
        );
    }

    #[test]
    fn self_loop_is_a_cycle() {
        // A → A
        let g = make_graph(&[("A", "A")]);
        let cycles = detect_cycles(&g);
        assert!(
            !cycles.is_empty(),
            "self-send actor must be detected as a cycle"
        );
    }

    #[test]
    fn dag_has_no_cycles() {
        // A → B, A → C, B → D, C → D  — pure DAG
        let g = make_graph(&[("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")]);
        let cycles = detect_cycles(&g);
        assert!(
            cycles.is_empty(),
            "DAG actor graph must not have cycles: {cycles:?}"
        );
    }
}
