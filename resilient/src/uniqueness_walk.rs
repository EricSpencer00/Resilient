//! Shared AST-walk helper used by the Ralph-Loop-Uniqueness feature
//! family. Each unique-feature pass needs to traverse a `Node` tree and
//! visit every sub-node — this helper centralizes the descent so each
//! feature module stays focused on its own rule.
//!
//! Visitor pattern: pre-order DFS, the closure runs on every node
//! (including the root). The walker descends into children regardless
//! of whether the closure looked at them — feature modules can decide
//! per-node whether to act.
//!
//! Why this lives in its own module: the family of unique features
//! (watchdog-feed, secret-erasure, transaction-commit, lock-ordering,
//! ISR-safety, …) shares the same descent and only differs in which
//! nodes they care about.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;

/// Pre-order traversal calling `f` on every node in the subtree, root first.
///
/// RES-1603: the `<'a>` lifetime parameter ties the closure's `&Node`
/// argument to the same lifetime as the input subtree. This lets
/// callers borrow sub-data (`&String` → `&'a str`, slices, etc.)
/// directly into structures that outlive the closure body — the
/// `pass_gate::Markers::scan` shared pre-scan is the first
/// consumer. Existing callers that pass closures whose body is
/// lifetime-agnostic (`|n| match n { ... }`) are unaffected because
/// the closure trait was inferred to accept any lifetime regardless.
pub(crate) fn visit<'a>(node: &'a Node, f: &mut impl FnMut(&'a Node)) {
    f(node);
    walk_children(node, f);
}

/// Apply `f` to each direct child of `node`, recursively descending.
pub(crate) fn walk_children<'a>(node: &'a Node, f: &mut impl FnMut(&'a Node)) {
    match node {
        Node::Program(items) => {
            for s in items {
                visit(&s.node, f);
            }
        }
        Node::Function { body, .. } => visit(body, f),
        Node::Block { stmts, .. } => {
            for s in stmts {
                visit(s, f);
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. } => visit(value, f),
        Node::ReturnStatement { value: Some(v), .. } => visit(v, f),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            visit(condition, f);
            visit(consequence, f);
            if let Some(alt) = alternative {
                visit(alt, f);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            visit(condition, f);
            visit(body, f);
        }
        Node::ForInStatement { iterable, body, .. } => {
            visit(iterable, f);
            visit(body, f);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            visit(function, f);
            for a in arguments {
                visit(a, f);
            }
        }
        Node::FieldAccess { target, .. } => visit(target, f),
        Node::FieldAssignment { target, value, .. } => {
            visit(target, f);
            visit(value, f);
        }
        Node::IndexExpression { target, index, .. } => {
            visit(target, f);
            visit(index, f);
        }
        Node::InfixExpression { left, right, .. } => {
            visit(left, f);
            visit(right, f);
        }
        Node::PrefixExpression { right, .. } => visit(right, f),
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                visit(i, f);
            }
        }
        Node::ExpressionStatement { expr, .. } => visit(expr, f),
        _ => {}
    }
}

/// Iterate every top-level Function in a Program, calling `f` with the
/// function's name, parameter list (type, name), and body.
pub(crate) fn for_each_function(
    program: &Node,
    mut f: impl FnMut(&str, &[(String, String)], &Node),
) {
    let Node::Program(stmts) = program else {
        return;
    };
    for stmt in stmts {
        if let Node::Function {
            name,
            parameters,
            body,
            ..
        } = &stmt.node
        {
            f(name, parameters, body);
        }
    }
}

/// Returns true if any sub-node satisfies `pred`.
///
/// RES-1238: walks the AST with **early termination** on the first
/// match. The previous implementation deferred to `visit`, which has
/// no escape hatch — once the predicate matched, the closure became
/// a no-op but the recursion still descended into every remaining
/// sub-node, paying the per-node match dispatch in `walk_children`.
/// For 20+ callsites across the typechecker (`secret_erasure`,
/// `sensor_freshness`, `idempotent_handler`, `backpressure_safe`,
/// `audit_log_required`, `degraded_mode`, `supervisor`, …) that was
/// a measurable amount of wasted AST traversal per typecheck.
///
/// The Node-variant dispatch below mirrors `walk_children` exactly
/// (same children visited in the same order) — the only difference
/// is that we propagate `true` upward as soon as `pred` matches,
/// short-circuiting siblings, aunts, and the rest of the tree.
pub(crate) fn any_node(node: &Node, mut pred: impl FnMut(&Node) -> bool) -> bool {
    any_node_inner(node, &mut pred)
}

fn any_node_inner(node: &Node, pred: &mut impl FnMut(&Node) -> bool) -> bool {
    if pred(node) {
        return true;
    }
    match node {
        Node::Program(items) => items.iter().any(|s| any_node_inner(&s.node, pred)),
        Node::Function { body, .. } => any_node_inner(body, pred),
        Node::Block { stmts, .. } => stmts.iter().any(|s| any_node_inner(s, pred)),
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. } => any_node_inner(value, pred),
        Node::ReturnStatement { value: Some(v), .. } => any_node_inner(v, pred),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            any_node_inner(condition, pred)
                || any_node_inner(consequence, pred)
                || alternative
                    .as_ref()
                    .is_some_and(|alt| any_node_inner(alt, pred))
        }
        Node::WhileStatement {
            condition, body, ..
        } => any_node_inner(condition, pred) || any_node_inner(body, pred),
        Node::ForInStatement { iterable, body, .. } => {
            any_node_inner(iterable, pred) || any_node_inner(body, pred)
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => any_node_inner(function, pred) || arguments.iter().any(|a| any_node_inner(a, pred)),
        Node::FieldAccess { target, .. } => any_node_inner(target, pred),
        Node::FieldAssignment { target, value, .. } => {
            any_node_inner(target, pred) || any_node_inner(value, pred)
        }
        Node::IndexExpression { target, index, .. } => {
            any_node_inner(target, pred) || any_node_inner(index, pred)
        }
        Node::InfixExpression { left, right, .. } => {
            any_node_inner(left, pred) || any_node_inner(right, pred)
        }
        Node::PrefixExpression { right, .. } => any_node_inner(right, pred),
        Node::ArrayLiteral { items, .. } => items.iter().any(|i| any_node_inner(i, pred)),
        Node::ExpressionStatement { expr, .. } => any_node_inner(expr, pred),
        _ => false,
    }
}

/// Returns true if `node` is `Node::Identifier { name == target }`.
#[allow(dead_code)]
pub(crate) fn is_ident(node: &Node, target: &str) -> bool {
    matches!(node, Node::Identifier { name, .. } if name == target)
}

/// Returns true if any of `targets` matches `node`'s identifier name.
#[allow(dead_code)]
pub(crate) fn ident_in(node: &Node, targets: &[&str]) -> bool {
    matches!(node, Node::Identifier { name, .. } if targets.contains(&name.as_str()))
}
