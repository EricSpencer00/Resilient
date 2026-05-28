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
        Node::Match {
            scrutinee, arms, ..
        } => {
            visit(scrutinee, f);
            for (_, guard, body) in arms {
                if let Some(g) = guard {
                    visit(g, f);
                }
                visit(body, f);
            }
        }
        // RES-2510: the following were missing, causing visitors to
        // silently skip sub-nodes inside these constructs.
        Node::FunctionLiteral { body, .. } => visit(body, f),
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            visit(condition, f);
            if let Some(m) = message {
                visit(m, f);
            }
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            visit(target, f);
            visit(index, f);
            visit(value, f);
        }
        Node::LetDestructureStruct { value, .. } | Node::LetTupleDestructure { value, .. } => {
            visit(value, f);
        }
        Node::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                visit(k, f);
                visit(v, f);
            }
        }
        Node::SetLiteral { items, .. } | Node::TupleLiteral { items, .. } => {
            for i in items {
                visit(i, f);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(b) = base {
                visit(b, f);
            }
            for (_, v) in fields {
                visit(v, f);
            }
        }
        Node::Slice { target, lo, hi, .. } => {
            visit(target, f);
            if let Some(l) = lo {
                visit(l, f);
            }
            if let Some(h) = hi {
                visit(h, f);
            }
        }
        Node::Range { lo, hi, .. } => {
            visit(lo, f);
            visit(hi, f);
        }
        Node::TupleIndex { tuple, .. } => visit(tuple, f),
        Node::InterpolatedString { parts, .. } => {
            for part in parts {
                if let crate::string_interp::StringPart::Expr(expr) = part {
                    visit(expr, f);
                }
            }
        }
        Node::TryCatch { body, handlers, .. } => {
            for s in body {
                visit(s, f);
            }
            for (_, handler_body) in handlers {
                for s in handler_body {
                    visit(s, f);
                }
            }
        }
        Node::TryExpression { expr, .. } => visit(expr, f),
        Node::NewtypeConstruct { value, .. } | Node::NamedArg { value, .. } => visit(value, f),
        Node::OptionalChain { object, access, .. } => {
            visit(object, f);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    visit(a, f);
                }
            }
        }
        Node::LiveBlock {
            body, invariants, ..
        } => {
            visit(body, f);
            for inv in invariants {
                visit(inv, f);
            }
        }
        Node::Quantifier { body, .. } => visit(body, f),
        // Leaf nodes and declarations without expression children.
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
        Node::Match {
            scrutinee, arms, ..
        } => {
            any_node_inner(scrutinee, pred)
                || arms.iter().any(|(_, guard, body)| {
                    guard.as_ref().is_some_and(|g| any_node_inner(g, pred))
                        || any_node_inner(body, pred)
                })
        }
        Node::FunctionLiteral { body, .. } => any_node_inner(body, pred),
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            any_node_inner(condition, pred)
                || message.as_ref().is_some_and(|m| any_node_inner(m, pred))
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            any_node_inner(target, pred)
                || any_node_inner(index, pred)
                || any_node_inner(value, pred)
        }
        Node::LetDestructureStruct { value, .. } | Node::LetTupleDestructure { value, .. } => {
            any_node_inner(value, pred)
        }
        Node::MapLiteral { entries, .. } => entries
            .iter()
            .any(|(k, v)| any_node_inner(k, pred) || any_node_inner(v, pred)),
        Node::SetLiteral { items, .. } | Node::TupleLiteral { items, .. } => {
            items.iter().any(|i| any_node_inner(i, pred))
        }
        Node::StructLiteral { fields, base, .. } => {
            base.as_ref().is_some_and(|b| any_node_inner(b, pred))
                || fields.iter().any(|(_, v)| any_node_inner(v, pred))
        }
        Node::Slice { target, lo, hi, .. } => {
            any_node_inner(target, pred)
                || lo.as_ref().is_some_and(|l| any_node_inner(l, pred))
                || hi.as_ref().is_some_and(|h| any_node_inner(h, pred))
        }
        Node::Range { lo, hi, .. } => any_node_inner(lo, pred) || any_node_inner(hi, pred),
        Node::TupleIndex { tuple, .. } => any_node_inner(tuple, pred),
        Node::InterpolatedString { parts, .. } => parts.iter().any(|part| {
            if let crate::string_interp::StringPart::Expr(expr) = part {
                any_node_inner(expr, pred)
            } else {
                false
            }
        }),
        Node::TryCatch { body, handlers, .. } => {
            body.iter().any(|s| any_node_inner(s, pred))
                || handlers
                    .iter()
                    .any(|(_, hb)| hb.iter().any(|s| any_node_inner(s, pred)))
        }
        Node::TryExpression { expr, .. } => any_node_inner(expr, pred),
        Node::NewtypeConstruct { value, .. } | Node::NamedArg { value, .. } => {
            any_node_inner(value, pred)
        }
        Node::OptionalChain { object, access, .. } => {
            any_node_inner(object, pred)
                || if let crate::ChainAccess::Method(_, args) = access {
                    args.iter().any(|a| any_node_inner(a, pred))
                } else {
                    false
                }
        }
        Node::LiveBlock {
            body, invariants, ..
        } => any_node_inner(body, pred) || invariants.iter().any(|inv| any_node_inner(inv, pred)),
        Node::Quantifier { body, .. } => any_node_inner(body, pred),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn any_node_finds_identifier_in_expression() {
        // `x` appears as an identifier in the function call argument.
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = parse(src);
        let found = any_node(
            &prog,
            |n| matches!(n, Node::Identifier { name, .. } if name == "x"),
        );
        assert!(
            found,
            "any_node must find the identifier 'x' in function body"
        );
    }

    #[test]
    fn any_node_returns_false_when_not_present() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = parse(src);
        let found = any_node(
            &prog,
            |n| matches!(n, Node::Identifier { name, .. } if name == "zzz"),
        );
        assert!(
            !found,
            "any_node must return false when identifier is absent"
        );
    }

    #[test]
    fn any_node_finds_infix_expression() {
        let src = "fn f(int x) -> int { return x + 1; }\n";
        let (prog, _) = parse(src);
        let found = any_node(&prog, |n| matches!(n, Node::InfixExpression { .. }));
        assert!(found, "any_node must find InfixExpression in function body");
    }

    #[test]
    fn for_each_function_visits_top_level_fns() {
        let src = "fn foo(int x) -> int { return x; }\nfn bar(int y) -> int { return y; }\n";
        let (prog, _) = parse(src);
        let mut names = Vec::new();
        for_each_function(&prog, |name, _params, _body| names.push(name.to_string()));
        assert!(names.contains(&"foo".to_string()));
        assert!(names.contains(&"bar".to_string()));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn for_each_function_empty_program() {
        let (prog, _) = parse("");
        let mut count = 0usize;
        for_each_function(&prog, |_, _, _| count += 1);
        assert_eq!(count, 0, "empty program has no functions to visit");
    }

    #[test]
    fn visit_reaches_nested_nodes() {
        let src = "let x = 1 + 2;\n";
        let (prog, _) = parse(src);
        let mut saw_infix = false;
        visit(&prog, &mut |n| {
            if matches!(n, Node::InfixExpression { .. }) {
                saw_infix = true;
            }
        });
        assert!(saw_infix, "visit must reach nested InfixExpression nodes");
    }

    #[test]
    fn any_node_descends_into_match_arms() {
        let src = "fn f(int x) -> int {\n\
                        return match x {\n\
                            1 => x + 42,\n\
                            _ => 0,\n\
                        };\n\
                    }\n";
        let (prog, _) = parse(src);
        let found = any_node(&prog, |n| {
            matches!(n, Node::IntegerLiteral { value: 42, .. })
        });
        assert!(found, "any_node must descend into match arm bodies");
    }

    #[test]
    fn is_ident_returns_true_for_match() {
        use crate::span::Span;
        let node = Node::Identifier {
            name: "myVar".to_string(),
            span: Span::default(),
        };
        assert!(is_ident(&node, "myVar"));
        assert!(!is_ident(&node, "other"));
    }
}
