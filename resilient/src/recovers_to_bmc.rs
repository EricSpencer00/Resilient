// RES-392b: Per-prefix bounded model checking for crash-recovery semantics.
//
// Extends RES-392 (MVP: final-state only) with verification that the
// recovers_to postcondition P holds after recovery from ANY instruction
// boundary in the function.
//
// For each control-flow prefix (every instruction boundary in the CFG),
// emits a Z3 obligation:
//   ∃ prefix_state ∈ reachable(fn_body[0..i]):
//     ¬(init(prefix_state) => P)
// If Z3 finds a satisfying prefix_state, report the specific instruction
// where recovery cannot be guaranteed.

use crate::{Node, Span};
use std::collections::HashMap;

/// Represents a control-flow node in the function's CFG.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CfgNode {
    /// Unique identifier for this node
    id: usize,
    /// The AST node this represents (or None for synthetic nodes like entry/exit)
    node: Option<Box<Node>>,
    /// Source span for diagnostics
    span: Span,
    /// Outgoing edges: (successor_id, edge_kind)
    successors: Vec<(usize, EdgeKind)>,
}

/// Represents different types of control-flow edges
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum EdgeKind {
    /// Normal sequential execution
    Fallthrough,
    /// Branch taken (if/match true case)
    Branch,
    /// Loop back edge
    Loop,
    /// Exception/early return
    Exception,
}

/// Control-flow graph for a function body
#[derive(Debug)]
struct ControlFlowGraph {
    /// All nodes in the graph, indexed by id
    #[allow(dead_code)]
    nodes: HashMap<usize, CfgNode>,
    /// Entry node ID
    #[allow(dead_code)]
    entry: usize,
    /// Exit node ID
    #[allow(dead_code)]
    exit: usize,
    /// Instruction boundary markers — one per statement-level node,
    /// ordered by control-flow position.
    prefix_boundaries: Vec<(usize, Span)>,
}

/// Builder state threaded through the recursive CFG construction.
struct Builder {
    nodes: HashMap<usize, CfgNode>,
    boundaries: Vec<(usize, Span)>,
    next_id: usize,
    exit_id: usize,
}

impl Builder {
    fn new_node(&mut self, node: Option<Box<Node>>, span: Span) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.insert(
            id,
            CfgNode {
                id,
                node,
                span,
                successors: Vec::new(),
            },
        );
        id
    }

    fn add_edge(&mut self, from: usize, to: usize, kind: EdgeKind) {
        if let Some(n) = self.nodes.get_mut(&from) {
            n.successors.push((to, kind));
        }
    }

    /// Build a linear chain of nodes from a list of statements.
    /// Returns the id of the last node in the chain (or `successor` if
    /// the list is empty) so callers can connect the continuation.
    ///
    /// Each statement node is recorded as a prefix boundary.
    fn build_stmts(&mut self, stmts: &[Node], successor: usize) -> usize {
        if stmts.is_empty() {
            return successor;
        }
        // Process in reverse so each node knows its successor.
        let mut next = successor;
        for stmt in stmts.iter().rev() {
            next = self.build_node(stmt, next);
        }
        next
    }

    /// Build a CFG sub-graph for a single AST node, connecting its last
    /// outgoing edge to `successor`. Returns the id of the node that is
    /// entered first when control reaches this sub-graph.
    fn build_node(&mut self, node: &Node, successor: usize) -> usize {
        let span = node_span(node);

        match node {
            Node::Block { stmts, .. } => {
                // Build the block's statements as a chain.
                self.build_stmts(stmts.as_slice(), successor)
            }

            Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
                // consequence branch
                let cons_entry = self.build_node(consequence, successor);
                // alternative branch (falls through to successor if absent)
                let alt_entry = match alternative {
                    Some(alt) => self.build_node(alt, successor),
                    None => successor,
                };
                // condition node — taken (Branch) goes to consequence,
                // not-taken (Fallthrough) goes to alternative / successor.
                let cond_id = self.new_node(Some(Box::new(*condition.clone())), span);
                self.boundaries.push((cond_id, span));
                self.add_edge(cond_id, cons_entry, EdgeKind::Branch);
                self.add_edge(cond_id, alt_entry, EdgeKind::Fallthrough);
                cond_id
            }

            Node::WhileStatement {
                condition, body, ..
            } => {
                // Header node evaluates the condition.
                let header = self.new_node(Some(Box::new(*condition.clone())), span);
                self.boundaries.push((header, span));
                // Body: its last node loops back to the header.
                let body_entry = self.build_node(body, header);
                // header → body (Branch) or falls through to successor
                // when condition is false.
                self.add_edge(header, body_entry, EdgeKind::Branch);
                self.add_edge(header, successor, EdgeKind::Fallthrough);
                // Also mark the body→header back edge as Loop.
                // (The body's last successor already points to `header`
                // via the build_node call above.)
                header
            }

            Node::ForInStatement { iterable, body, .. } => {
                // Treat like while: iterable evaluation + body with back edge.
                let header = self.new_node(Some(Box::new(*iterable.clone())), span);
                self.boundaries.push((header, span));
                let body_entry = self.build_node(body, header);
                self.add_edge(header, body_entry, EdgeKind::Branch);
                self.add_edge(header, successor, EdgeKind::Fallthrough);
                header
            }

            Node::ReturnStatement { .. } => {
                // Return always transfers to the exit; the normal
                // `successor` is unreachable from here.
                let ret_id = self.new_node(Some(Box::new(node.clone())), span);
                self.boundaries.push((ret_id, span));
                self.add_edge(ret_id, self.exit_id, EdgeKind::Exception);
                ret_id
            }

            Node::Match { arms, .. } => {
                // Each arm is an independent branch from the match node.
                // Arms are (Pattern, Option<guard>, body_Node) tuples.
                let match_id = self.new_node(Some(Box::new(node.clone())), span);
                self.boundaries.push((match_id, span));
                for arm in arms {
                    let arm_entry = self.build_node(&arm.2, successor);
                    self.add_edge(match_id, arm_entry, EdgeKind::Branch);
                }
                // Fallthrough when no arm matches (should not happen in
                // exhaustive match, but keeps the graph well-formed).
                self.add_edge(match_id, successor, EdgeKind::Fallthrough);
                match_id
            }

            // Any other statement (let, expr-stmt, assignment, etc.)
            // is a single basic-block node.
            _ => {
                let id = self.new_node(Some(Box::new(node.clone())), span);
                self.boundaries.push((id, span));
                self.add_edge(id, successor, EdgeKind::Fallthrough);
                id
            }
        }
    }
}

/// Extract the span from an AST node (best-effort).
fn node_span(node: &Node) -> Span {
    match node {
        Node::Block { span, .. }
        | Node::IfStatement { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::LetStatement { span, .. }
        | Node::ExpressionStatement { span, .. }
        | Node::Function { span, .. }
        | Node::Match { span, .. } => *span,
        _ => Span::default(),
    }
}

impl ControlFlowGraph {
    /// Build a proper statement-level CFG from a function body.
    ///
    /// Each statement, if-condition, loop-header, and return becomes its own
    /// node and prefix boundary.  This replaces the previous MVP that treated
    /// the entire body as one opaque node, giving Phase 2 BMC verification
    /// per-instruction granularity when it is implemented.
    fn from_body(body: &Node) -> Self {
        // Reserve id=0 for entry, id=1 for exit.
        let mut b = Builder {
            nodes: HashMap::new(),
            boundaries: Vec::new(),
            next_id: 2,
            exit_id: 1,
        };

        // Synthetic entry node
        b.nodes.insert(
            0,
            CfgNode {
                id: 0,
                node: None,
                span: Span::default(),
                successors: Vec::new(),
            },
        );
        // Synthetic exit node
        b.nodes.insert(
            1,
            CfgNode {
                id: 1,
                node: None,
                span: Span::default(),
                successors: Vec::new(),
            },
        );

        // Build the body sub-graph. The continuation of the entire body is
        // the exit node (normal fall-off).
        let body_entry = b.build_node(body, 1 /* exit */);

        // Wire entry → body.
        b.nodes
            .get_mut(&0)
            .expect("entry always present")
            .successors
            .push((body_entry, EdgeKind::Fallthrough));

        ControlFlowGraph {
            nodes: b.nodes,
            entry: 0,
            exit: 1,
            prefix_boundaries: b.boundaries,
        }
    }

    /// Enumerate all instruction-boundary prefixes in the CFG.
    fn enumerate_prefixes(&self) -> Vec<(usize, Span)> {
        self.prefix_boundaries.clone()
    }
}

/// Generate Z3 SMT-LIB2 obligation for per-prefix recovery invariant.
///
/// RES-392b Phase 2 — not yet implemented. Returns an empty string so
/// the Phase 3 loop can iterate without changes when Phase 2 lands.
#[allow(dead_code)]
fn generate_prefix_obligation(
    prefix_id: usize,
    _init_state: &str,
    recovers_clause: &Node,
) -> String {
    // TODO: RES-392b Phase 2 — emit:
    //   (push)
    //   (assert (not (=> init_<prefix_id> <recovers_clause_z3>)))
    //   (check-sat)
    let _ = (prefix_id, recovers_clause);
    String::new()
}

/// Check crash-recovery guarantees for a function's recovers_to clause
/// via per-prefix bounded model checking.
///
/// Phase 1 now produces a proper statement-level CFG with per-instruction
/// boundaries for if/else, while, for, return, and match.  Phase 2/3 Z3
/// integration is still pending (always returns Ok); the infrastructure is
/// in place for when the solver is wired up.
pub(crate) fn check_recovers_to_bmc(
    _fn_name: &str,
    fn_body: &Node,
    _recovers_clause: &Node,
) -> Result<(), String> {
    let cfg = ControlFlowGraph::from_body(fn_body);
    let prefixes = cfg.enumerate_prefixes();

    for (prefix_id, _span) in prefixes {
        let _obligation = generate_prefix_obligation(prefix_id, "init_state", _recovers_clause);
        // TODO: RES-392b Phase 2 — invoke Z3 solver on `_obligation`.
        // On SAT: return Err with the failing prefix span.
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn body_of(src: &str) -> Node {
        let (prog, _) = parse(src);
        match prog {
            Node::Program(stmts) => match &stmts[0].node {
                Node::Function { body, .. } => *body.clone(),
                other => panic!("expected Function, got {:?}", other),
            },
            other => panic!("expected Program, got {:?}", other),
        }
    }

    #[test]
    fn linear_body_produces_one_boundary_per_statement() {
        let body = body_of("fn f(int x) -> int { let a = 1; let b = 2; return a; }");
        let cfg = ControlFlowGraph::from_body(&body);
        let prefixes = cfg.enumerate_prefixes();
        // Three statements → at least 3 boundaries.
        assert!(
            prefixes.len() >= 3,
            "expected ≥3 boundaries, got {}",
            prefixes.len()
        );
    }

    #[test]
    fn if_else_produces_extra_boundaries() {
        let body = body_of("fn f(int x) -> int { if x > 0 { return 1; } else { return 0; } }");
        let cfg = ControlFlowGraph::from_body(&body);
        // Condition node + two return nodes = at least 3 boundaries.
        let prefixes = cfg.enumerate_prefixes();
        assert!(
            prefixes.len() >= 3,
            "if/else must produce ≥3 boundaries, got {}",
            prefixes.len()
        );
    }

    #[test]
    fn while_loop_produces_header_boundary() {
        let body = body_of("fn f(int x) -> int { while x > 0 { let _y = x; } return x; }");
        let cfg = ControlFlowGraph::from_body(&body);
        let prefixes = cfg.enumerate_prefixes();
        // Header + let + return = at least 3 boundaries.
        assert!(
            prefixes.len() >= 3,
            "while loop must produce ≥3 boundaries, got {}",
            prefixes.len()
        );
    }

    #[test]
    fn bmc_check_returns_ok_for_simple_fn() {
        let body = body_of("fn f(int x) -> int { return x; }");
        let clause = Node::BooleanLiteral {
            value: true,
            span: Span::default(),
        };
        let result = check_recovers_to_bmc("f", &body, &clause);
        assert!(result.is_ok(), "stub must return Ok: {:?}", result);
    }
}
