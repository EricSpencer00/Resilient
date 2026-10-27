//! Incoming tail edges allow a trampoline group to end in a terminal member.

use crate::Node;
use std::collections::HashSet;

pub(super) fn incoming_tail_targets(program: &Node, annotated: &HashSet<&str>) -> HashSet<String> {
    let mut targets = HashSet::new();
    if let Node::Program(statements) = program {
        for statement in statements {
            if let Node::Function { name, body, .. } = &statement.node
                && annotated.contains(name.as_str())
            {
                collect(body, true, name, annotated, &mut targets);
            }
        }
    }
    targets
}

fn collect(
    node: &Node,
    tail: bool,
    caller: &str,
    annotated: &HashSet<&str>,
    targets: &mut HashSet<String>,
) {
    match node {
        Node::CallExpression { function, .. } if tail => {
            if let Node::Identifier { name, .. } = function.as_ref()
                && name != caller
                && annotated.contains(name.as_str())
            {
                targets.insert(name.clone());
            }
        }
        Node::Block { stmts, .. } => {
            for (index, statement) in stmts.iter().enumerate() {
                collect(
                    statement,
                    tail && index + 1 == stmts.len(),
                    caller,
                    annotated,
                    targets,
                );
            }
        }
        Node::ReturnStatement {
            value: Some(value), ..
        } => {
            collect(value, true, caller, annotated, targets);
        }
        Node::ExpressionStatement { expr, .. } => collect(expr, tail, caller, annotated, targets),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            collect(consequence, tail, caller, annotated, targets);
            if let Some(alternative) = alternative {
                collect(alternative, tail, caller, annotated, targets);
            }
        }
        Node::Match { arms, .. } => {
            for (_, _, body) in arms {
                collect(body, tail, caller, annotated, targets);
            }
        }
        // Closures and nested declarations own separate return contexts.
        _ => {}
    }
}
