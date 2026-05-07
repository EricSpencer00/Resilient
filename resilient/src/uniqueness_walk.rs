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
pub(crate) fn visit(node: &Node, f: &mut impl FnMut(&Node)) {
    f(node);
    walk_children(node, f);
}

/// Apply `f` to each direct child of `node`, recursively descending.
pub(crate) fn walk_children(node: &Node, f: &mut impl FnMut(&Node)) {
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
pub(crate) fn any_node(node: &Node, mut pred: impl FnMut(&Node) -> bool) -> bool {
    let mut found = false;
    visit(node, &mut |n| {
        if !found && pred(n) {
            found = true;
        }
    });
    found
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
