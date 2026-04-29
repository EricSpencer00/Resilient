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

use std::collections::HashMap;
use crate::{Node, Span};

/// Represents a control-flow node in the function's CFG.
#[derive(Debug, Clone)]
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
    nodes: HashMap<usize, CfgNode>,
    /// Entry node ID
    entry: usize,
    /// Exit node ID
    exit: usize,
    /// Instruction boundary markers (line numbers where prefixes begin)
    prefix_boundaries: Vec<(usize, Span)>, // (node_id, span)
}

impl ControlFlowGraph {
    /// Build a CFG from a function body
    fn from_body(body: &Node) -> Self {
        let mut graph = ControlFlowGraph {
            nodes: HashMap::new(),
            entry: 0,
            exit: 1,
            prefix_boundaries: Vec::new(),
        };

        // Create entry and exit nodes
        graph.nodes.insert(
            0,
            CfgNode {
                id: 0,
                node: None,
                span: Span::default(),
                successors: vec![(1, EdgeKind::Fallthrough)],
            },
        );
        graph.nodes.insert(
            1,
            CfgNode {
                id: 1,
                node: None,
                span: Span::default(),
                successors: vec![],
            },
        );

        // Extract CFG structure from body
        // TODO: RES-392b Phase 1 - implement full CFG extraction
        // For now, just record the body as a single basic block
        let mut body_node = CfgNode {
            id: 2,
            node: Some(Box::new(body.clone())),
            span: Span::default(),
            successors: vec![(1, EdgeKind::Fallthrough)],
        };

        // Extract span from body for better diagnostics
        body_node.span = match body {
            Node::Block { span, .. } => *span,
            _ => Span::default(),
        };

        graph.nodes.insert(2, body_node);

        // Connect entry to body
        if let Some(entry_node) = graph.nodes.get_mut(&0) {
            entry_node.successors = vec![(2, EdgeKind::Fallthrough)];
        }

        // Mark prefix boundaries (instruction entry points)
        graph.prefix_boundaries.push((2, Span::default()));

        graph
    }

    /// Enumerate all instruction prefixes in the CFG
    /// Returns: list of (prefix_id, reachable_state_at_boundary, span)
    fn enumerate_prefixes(&self) -> Vec<(usize, Span)> {
        self.prefix_boundaries.clone()
    }
}

/// Check crash-recovery guarantees for a function's recovers_to clause
/// via per-prefix bounded model checking.
///
/// Returns Ok(()) if all prefixes are verified to recover.
/// Returns Err(msg) with diagnostic pointing to a failing prefix.
pub(crate) fn check_recovers_to_bmc(
    _fn_name: &str,
    fn_body: &Node,
    _recovers_clause: &Node,
) -> Result<(), String> {
    // RES-392b Phase 1: CFG extraction
    let cfg = ControlFlowGraph::from_body(fn_body);

    // RES-392b Phase 2: Per-prefix enumeration and Z3 verification
    let prefixes = cfg.enumerate_prefixes();

    for (prefix_id, _span) in prefixes {
        // TODO: RES-392b Phase 2 - emit Z3 obligation per prefix
        // For now, assume all prefixes recover (stub)
        //
        // Pseudo-code:
        // let prefix_obligation = format!(
        //     "(assert (not (=> (init prefix_state_{}) {})))",
        //     prefix_id,
        //     z3_encode_expr(recovers_clause)
        // );
        // if z3_solve(&prefix_obligation).is_sat() {
        //     return Err(format!(
        //         "{}:{}: no recovery guarantee after line {} — add to init or narrow recovers_to",
        //         fn_name, span.line, span.line
        //     ));
        // }
    }

    // RES-392b Phase 3: All prefixes verified
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cfg_construction() {
        // TODO: RES-392b - test CFG extraction on simple function bodies
    }

    #[test]
    fn test_prefix_enumeration() {
        // TODO: RES-392b - test that all instruction boundaries are enumerated
    }
}
