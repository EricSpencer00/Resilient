//! Execution-only lowering: generated guards must never earn source proof credit.
use crate::{Node, span::Spanned};
use std::collections::HashMap;

const PREFIX: &str = "$loop_bound$";

#[cfg(feature = "jit")]
pub(crate) fn needs_guard(program: &Node) -> bool {
    let bounds = crate::loop_bound::collect();
    crate::uniqueness_walk::any_node(program, |node| match node {
        Node::Function { name, .. } => bounds.contains_key(name),
        Node::Identifier { name, .. } => name.starts_with(PREFIX),
        _ => false,
    })
}

pub(crate) fn lower_statements(statements: &[Spanned<Node>]) -> Option<Vec<Spanned<Node>>> {
    if crate::loop_bound::collect().is_empty() {
        return None;
    }
    match lower(&Node::Program(statements.to_vec()))? {
        Node::Program(statements) => Some(statements),
        _ => None,
    }
}

pub(crate) fn lower(program: &Node) -> Option<Node> {
    let bounds = crate::loop_bound::collect();
    if bounds.is_empty() {
        return None;
    }
    let mut result = program.clone();
    let mut serial = 0;
    lower_node(&mut result, None, &bounds, &mut serial);
    Some(result)
}

fn lower_node(
    node: &mut Node,
    inherited: Option<u32>,
    bounds: &HashMap<String, crate::loop_bound::LoopBoundSpec>,
    serial: &mut usize,
) {
    // A generated wrapper is already executable, including all its descendants.
    if let Node::Block { stmts, .. } = node
        && matches!(stmts.first(), Some(Node::LetStatement { name, .. }) if name.starts_with(PREFIX))
    {
        return;
    }
    let bound = match node {
        Node::Function { name, .. } => bounds.get(name).map(|s| s.bound).or(inherited),
        _ => inherited,
    };
    crate::uniqueness_walk::children_mut(node, &mut |child| {
        lower_node(child, bound, bounds, serial)
    });
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

// Public in-memory invocations must not consume another invocation's attributes.
thread_local! {
    static INVOCATION: std::cell::RefCell<Option<HashMap<String, crate::loop_bound::LoopBoundSpec>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) struct Invocation(Option<HashMap<String, crate::loop_bound::LoopBoundSpec>>);
impl Invocation {
    pub(crate) fn enter() -> Self {
        Self(INVOCATION.with(|slot| slot.replace(Some(HashMap::new()))))
    }
}
impl Drop for Invocation {
    fn drop(&mut self) {
        INVOCATION.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

pub(crate) fn scoped_bounds() -> Option<HashMap<String, crate::loop_bound::LoopBoundSpec>> {
    INVOCATION.with(|slot| slot.borrow().clone())
}

pub(crate) fn record_scoped(name: &str, record: &crate::feature_attrs::AttrRecord) {
    if record.name != "loop_bound" {
        return;
    }
    let Ok(bound) = record.args.trim().parse::<u32>() else {
        return;
    };
    if bound == 0 {
        return;
    }
    INVOCATION.with(|slot| {
        if let Some(bounds) = slot.borrow_mut().as_mut() {
            bounds.insert(name.to_owned(), crate::loop_bound::LoopBoundSpec { bound });
        }
    });
}
