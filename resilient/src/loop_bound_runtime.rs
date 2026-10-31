//! Execution-only lowering: generated guards must never earn source proof credit.
use crate::{Node, span::Spanned};
use std::collections::HashMap;

const PREFIX: &str = "$loop_bound$";

#[cfg(feature = "jit")]
pub(crate) fn needs_guard(program: &Node) -> bool {
    let bounds = crate::loop_bound::collect();
    let declarations = crate::loop_bound_metadata::snapshot();
    let mut needed = false;
    crate::loop_bound_metadata::walk(program, &mut Vec::new(), &mut |node, path| {
        needed |= match node {
            Node::Function { name, .. } => match &declarations {
                Some(declarations) => declarations
                    .get(path)
                    .and_then(|attributes| attributes.loop_bound)
                    .is_some(),
                None => bounds.contains_key(name),
            },
            Node::Identifier { name, .. } => name.starts_with(PREFIX),
            _ => false,
        };
    });
    needed
}

pub(crate) fn lower_statements(statements: &[Spanned<Node>]) -> Option<Vec<Spanned<Node>>> {
    match lower(&Node::Program(statements.to_vec()))? {
        Node::Program(statements) => Some(statements),
        _ => None,
    }
}

pub(crate) fn lower(program: &Node) -> Option<Node> {
    let bounds = crate::loop_bound::collect();
    let declarations = crate::loop_bound_metadata::snapshot();
    if bounds.is_empty()
        && !declarations.as_ref().is_some_and(|items| {
            items
                .values()
                .any(|attributes| attributes.loop_bound.is_some())
        })
    {
        return None;
    }
    let mut result = program.clone();
    let mut serial = 0;
    lower_node(
        &mut result,
        None,
        &bounds,
        &declarations,
        &mut Vec::new(),
        &mut serial,
    );
    Some(result)
}

fn lower_node(
    node: &mut Node,
    inherited: Option<u32>,
    bounds: &HashMap<String, crate::loop_bound::LoopBoundSpec>,
    declarations: &Option<crate::loop_bound_metadata::Bounds>,
    path: &mut crate::loop_bound_metadata::DeclarationPath,
    serial: &mut usize,
) {
    // A generated wrapper is already executable, including all its descendants.
    if let Node::Block { stmts, .. } = node
        && matches!(stmts.first(), Some(Node::LetStatement { name, .. }) if name.starts_with(PREFIX))
    {
        return;
    }
    let own = crate::loop_bound_metadata::declaration(node);
    if let Some(own) = &own {
        path.push(own.clone());
    }
    let bound = match node {
        Node::Function { name, .. } => match declarations {
            Some(declarations) => declarations
                .get(path)
                .and_then(|attributes| attributes.loop_bound),
            None => bounds.get(name).copied(),
        }
        .map(|s| s.bound)
        .or(inherited),
        _ => inherited,
    };
    crate::uniqueness_walk::children_mut(node, &mut |child| {
        lower_node(child, bound, bounds, declarations, path, serial)
    });
    if own.is_some() {
        path.pop();
    }
    let Some(limit) = bound else { return };
    let Node::WhileStatement { body, span, .. } = node else {
        return;
    };
    let span = *span;
    let name = format!("{PREFIX}{serial}");
    *serial += 1;
    let id = || Node::Identifier {
        name: name.clone(),
        span,
    };
    let int = |value| Node::IntegerLiteral { value, span };
    let guard = Node::Assert {
        condition: Box::new(Node::InfixExpression {
            left: Box::new(id()),
            operator: "<",
            right: Box::new(int(i64::from(limit))),
            span,
        }),
        message: Some(Box::new(Node::StringLiteral {
            value: format!("loop bound exceeded: {limit} iterations"),
            span,
        })),
        span,
    };
    let increment = Node::Assignment {
        name: name.clone(),
        value: Box::new(Node::InfixExpression {
            left: Box::new(id()),
            operator: "+",
            right: Box::new(int(1)),
            span,
        }),
        span,
    };
    let original_body = std::mem::replace(
        body.as_mut(),
        Node::Block {
            stmts: Vec::new(),
            span,
        },
    );
    **body = Node::Block {
        stmts: vec![guard, increment, original_body],
        span,
    };
    let original_loop = std::mem::replace(
        node,
        Node::Block {
            stmts: Vec::new(),
            span,
        },
    );
    *node = Node::Block {
        stmts: vec![
            Node::LetStatement {
                name,
                value: Box::new(int(0)),
                type_annot: None,
                span,
                is_const: false,
            },
            original_loop,
        ],
        span,
    };
}

pub(crate) use crate::loop_bound_metadata::Invocation;
