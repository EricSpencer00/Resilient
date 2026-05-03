//! RES-398: termination checking — recursive fns require an explicit
//! `// @decreases <metric>` clause or `// @may_diverge` escape hatch.
//!
//! Reddit critique
//! (https://www.reddit.com/r/VibeCodersNest/comments/1ssv8ih/) asks:
//! *can an invalid state even be expressed in your system?* Today,
//! unbounded recursion is one such expressible-but-invalid state.
//! Resilient targets safety-critical embedded systems where
//! unbounded recursion is a known hazard (stack overflow on
//! Cortex-M, no graceful recovery).
//!
//! The runtime depth cap (RES-267) catches it at runtime. That's
//! *filtering*, not structural enforcement. This pass closes the
//! gap on direct (self-call) recursion: every directly-recursive
//! function must declare *either* a decreasing metric or that
//! divergence is acceptable.
//!
//! # Surface syntax
//!
//! Annotation goes on the line *immediately above* the `fn` keyword:
//!
//! ```text
//! // @decreases n
//! fn fact(int n) requires n >= 0 {
//!     if n <= 1 { return 1; }
//!     return n * fact(n - 1);
//! }
//!
//! // @may_diverge
//! fn event_loop() {
//!     loop_forever();
//! }
//! ```
//!
//! ## Why comment-based and not a new keyword?
//!
//! Comment-based annotation keeps the surface change minimal — no
//! new tokens, no parser arms, no AST node. The same line-offset
//! convention is already used by `// resilient: allow LXXXX` and
//! `// source:` (RES-397). A future ticket can promote this to a
//! first-class clause once the design has stabilized.
//!
//! # Behavior
//!
//! - **Default: off.** Existing programs are not affected. The
//!   pass returns `Ok(())` immediately when strict mode is not
//!   enabled, so cross-compile, REPL, and `rz run` are untouched.
//! - **Opt-in via `--strict-termination`** (CLI). When set,
//!   unannotated direct recursion produces a typechecker error.
//!
//! # Out of scope (future tickets)
//!
//! - **Z3 verification of the `decreases` metric**: the syntactic
//!   check lands first; SMT proof of strict decrease is a separate
//!   ticket.
//! - **Loop termination**: `while`/`for` are out of scope here;
//!   loop invariants (RES-132a) and loop-bound checks live elsewhere.

use crate::Node;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

/// RES-398: process-wide flag controlling whether the termination
/// check is enforced. Off by default — existing programs see no
/// change. Mirrors the `bounds_check::DENY_UNPROVEN_BOUNDS` pattern.
static STRICT_TERMINATION: AtomicBool = AtomicBool::new(false);

/// Enable `--strict-termination` mode. Called from `main.rs` CLI
/// parsing before `check_program_with_source` runs.
pub fn set_strict_termination(on: bool) {
    STRICT_TERMINATION.store(on, Ordering::Relaxed);
}

fn strict_termination() -> bool {
    STRICT_TERMINATION.load(Ordering::Relaxed)
}

/// RES-398 + RES-774: typechecker extension pass — for every function
/// that participates in a recursive SCC (directly recursive or mutually
/// recursive), require either `// @decreases <metric>` or
/// `// @may_diverge` on the line above the `fn` keyword. No-op
/// when strict mode is off.
pub fn check(program: &Node, source_path: &str) -> Result<(), String> {
    if !strict_termination() {
        return Ok(());
    }
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    let source = std::fs::read_to_string(source_path).unwrap_or_default();
    let lines: Vec<&str> = source.lines().collect();

    // Build call graph: fn_name -> set of functions it calls
    let mut call_graph: HashMap<String, HashSet<String>> = HashMap::new();
    let mut fn_info: HashMap<String, (usize, usize)> = HashMap::new(); // name -> (line, col)

    for spanned in stmts {
        if let Node::Function {
            name, body, span, ..
        } = &spanned.node
        {
            if name.is_empty() {
                continue;
            }
            fn_info.insert(name.clone(), (span.start.line, span.start.column));
            let mut callees = HashSet::new();
            collect_calls(body, &mut callees);
            call_graph.insert(name.clone(), callees);
        }
    }

    // Find SCCs using Tarjan's algorithm
    let sccs = find_sccs(&call_graph);

    // Check each SCC: if it has cycles, all functions in it need annotations
    for scc in sccs {
        if scc.len() == 1 {
            // Single function: check if it has cycles (self-calls)
            let name = &scc[0];
            if !call_graph
                .get(name)
                .map(|calls| calls.contains(name))
                .unwrap_or(false)
            {
                continue; // No recursion
            }
        } else {
            // Multiple functions: check if any edge within SCC exists
            let scc_set: HashSet<&str> = scc.iter().map(|s| s.as_str()).collect();
            let has_cycle = scc.iter().any(|name| {
                call_graph
                    .get(name)
                    .map(|calls| calls.iter().any(|c| scc_set.contains(c.as_str())))
                    .unwrap_or(false)
            });
            if !has_cycle {
                continue;
            }
        }

        // This SCC is recursive; check annotations for all functions in it
        for name in &scc {
            if let Some((fn_line, col)) = fn_info.get(name) {
                if *fn_line < 2 {
                    let recursion_type = if scc.len() == 1 {
                        "directly recursive"
                    } else {
                        "mutually recursive"
                    };
                    return Err(format!(
                        "{}:{}:{}: error: function `{}` is {} but has no termination annotation; \
                         expected `// @decreases <metric>` or `// @may_diverge` on the line above",
                        source_path, fn_line, col, name, recursion_type
                    ));
                }
                let prev = lines.get(fn_line - 2).copied().unwrap_or("");
                if !has_termination_annotation(prev) {
                    let recursion_type = if scc.len() == 1 {
                        "directly recursive"
                    } else {
                        "mutually recursive"
                    };
                    return Err(format!(
                        "{}:{}:{}: error: function `{}` is {} but has no termination annotation; \
                         expected `// @decreases <metric>` or `// @may_diverge` on the line above",
                        source_path, fn_line, col, name, recursion_type
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Returns true when `line` (a single source line, no newline) carries
/// a `// @decreases <metric>` or `// @may_diverge` annotation. Leading
/// whitespace is ignored; trailing text after the annotation keyword
/// is treated as the metric / comment payload and is not validated
/// here (a future Z3 ticket will check the metric strictly decreases).
fn has_termination_annotation(line: &str) -> bool {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("// @decreases") {
        // Require at least one non-whitespace char after the keyword.
        return !rest.trim().is_empty();
    }
    if let Some(rest) = trimmed.strip_prefix("// @may_diverge") {
        // `// @may_diverge` alone is sufficient; trailing comment OK.
        return rest.is_empty() || rest.starts_with(char::is_whitespace);
    }
    false
}

/// RES-774: collect all function names called by a node, adding them to
/// the `callees` set. Used to build the call graph for SCC analysis.
fn collect_calls(node: &Node, callees: &mut HashSet<String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                callees.insert(name.clone());
            }
            collect_calls(function, callees);
            for arg in arguments {
                collect_calls(arg, callees);
            }
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                collect_calls(stmt, callees);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_calls(condition, callees);
            collect_calls(consequence, callees);
            if let Some(alt) = alternative {
                collect_calls(alt, callees);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_calls(condition, callees);
            collect_calls(body, callees);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_calls(iterable, callees);
            collect_calls(body, callees);
        }
        Node::LiveBlock { body, .. } => collect_calls(body, callees),
        Node::ReturnStatement { value: Some(v), .. } => collect_calls(v, callees),
        Node::ExpressionStatement { expr, .. } => collect_calls(expr, callees),
        Node::LetStatement { value, .. } => collect_calls(value, callees),
        Node::InfixExpression { left, right, .. } => {
            collect_calls(left, callees);
            collect_calls(right, callees);
        }
        Node::PrefixExpression { right, .. } => collect_calls(right, callees),
        Node::FieldAccess { target, .. } => collect_calls(target, callees),
        _ => {}
    }
}

/// RES-774: Tarjan's algorithm for finding strongly connected components.
/// Returns a Vec of SCCs (each SCC is a Vec of function names).
fn find_sccs(graph: &HashMap<String, HashSet<String>>) -> Vec<Vec<String>> {
    let mut index_counter = 0;
    let mut stack: Vec<String> = Vec::new();
    let mut indices: HashMap<String, usize> = HashMap::new();
    let mut lowlinks: HashMap<String, usize> = HashMap::new();
    let mut on_stack: HashSet<String> = HashSet::new();
    let mut sccs: Vec<Vec<String>> = Vec::new();

    for node in graph.keys() {
        if !indices.contains_key(node) {
            strongconnect(
                node,
                &mut index_counter,
                &mut indices,
                &mut lowlinks,
                &mut stack,
                &mut on_stack,
                &mut sccs,
                graph,
            );
        }
    }
    sccs
}

#[allow(clippy::too_many_arguments)]
fn strongconnect(
    node: &str,
    index_counter: &mut usize,
    indices: &mut HashMap<String, usize>,
    lowlinks: &mut HashMap<String, usize>,
    stack: &mut Vec<String>,
    on_stack: &mut HashSet<String>,
    sccs: &mut Vec<Vec<String>>,
    graph: &HashMap<String, HashSet<String>>,
) {
    indices.insert(node.to_string(), *index_counter);
    lowlinks.insert(node.to_string(), *index_counter);
    *index_counter += 1;
    stack.push(node.to_string());
    on_stack.insert(node.to_string());

    if let Some(successors) = graph.get(node) {
        for successor in successors {
            if !indices.contains_key(successor) {
                strongconnect(
                    successor,
                    index_counter,
                    indices,
                    lowlinks,
                    stack,
                    on_stack,
                    sccs,
                    graph,
                );
                let succ_lowlink = *lowlinks.get(successor).unwrap_or(&0);
                lowlinks.insert(
                    node.to_string(),
                    (*lowlinks.get(node).unwrap_or(&0)).min(succ_lowlink),
                );
            } else if on_stack.contains(successor) {
                let succ_index = *indices.get(successor).unwrap_or(&0);
                lowlinks.insert(
                    node.to_string(),
                    (*lowlinks.get(node).unwrap_or(&0)).min(succ_index),
                );
            }
        }
    }

    if lowlinks.get(node) == indices.get(node) {
        let mut scc = Vec::new();
        loop {
            let w = stack.pop().unwrap();
            on_stack.remove(&w);
            scc.push(w.clone());
            if w == node {
                break;
            }
        }
        sccs.push(scc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotation_decreases_with_metric_accepted() {
        assert!(has_termination_annotation("// @decreases n"));
        assert!(has_termination_annotation("    // @decreases n - 1"));
        assert!(has_termination_annotation("// @decreases (a, b)"));
    }

    #[test]
    fn annotation_decreases_without_metric_rejected() {
        assert!(!has_termination_annotation("// @decreases"));
        assert!(!has_termination_annotation("// @decreases   "));
    }

    #[test]
    fn annotation_may_diverge_accepted() {
        assert!(has_termination_annotation("// @may_diverge"));
        assert!(has_termination_annotation("    // @may_diverge"));
        assert!(has_termination_annotation(
            "// @may_diverge — event loop is intentionally non-terminating"
        ));
    }

    #[test]
    fn unrelated_comment_rejected() {
        assert!(!has_termination_annotation("// just a comment"));
        assert!(!has_termination_annotation("// source: rfc-1234"));
        assert!(!has_termination_annotation(""));
    }

    // Note: the `check` function is exercised end-to-end via the
    // golden examples in `examples/termination_*.rz`. Unit-testing
    // it here would require constructing a full `Node::Program`
    // with span info, which the integration tests already do.
}
