//! Analysis-only match scopes; this tree is never compiled or executed.
use crate::{EnumPatternPayload, Node, Pattern};
use std::collections::BTreeSet;

fn bindings(pattern: &Pattern, names: &mut BTreeSet<String>) {
    match pattern {
        Pattern::Identifier(name) => {
            names.insert(name.clone());
        }
        Pattern::Bind(name, inner) => {
            names.insert(name.clone());
            bindings(inner, names);
        }
        Pattern::Some(inner) | Pattern::Ok(inner) | Pattern::Err(inner) => bindings(inner, names),
        Pattern::Or(items) | Pattern::Tuple(items) | Pattern::TupleStruct { fields: items, .. } => {
            for item in items {
                bindings(item, names);
            }
        }
        Pattern::Struct { fields, .. } => {
            for (_, pattern) in fields {
                bindings(pattern, names);
            }
        }
        Pattern::EnumVariant { payload, .. } => match payload {
            EnumPatternPayload::Named(fields) => {
                for (_, pattern) in fields {
                    bindings(pattern, names);
                }
            }
            EnumPatternPayload::Tuple(items) => {
                for item in items {
                    bindings(item, names);
                }
            }
            EnumPatternPayload::None => {}
        },
        Pattern::Literal(_) | Pattern::Wildcard | Pattern::Range { .. } | Pattern::None => {}
    }
}

fn returns_from_arm(node: &Node) -> bool {
    if matches!(node, Node::Function { .. } | Node::FunctionLiteral { .. }) {
        return false;
    }
    let mut found = matches!(node, Node::ReturnStatement { .. });
    crate::uniqueness_walk::children(node, &mut |child| found |= returns_from_arm(child));
    found
}

pub(super) fn scoped(program: &Node) -> Node {
    fn rewrite(node: &mut Node) {
        crate::uniqueness_walk::children_mut(node, &mut rewrite);
        let Node::Match {
            scrutinee,
            arms,
            span,
        } = node
        else {
            return;
        };
        let span = *span;
        let exits = arms.iter().any(|(_, _, body)| returns_from_arm(body));
        let mut branches = Vec::new();
        for (pattern, guard, body) in arms {
            let mut names = BTreeSet::new();
            bindings(pattern, &mut names);
            let arguments = names
                .iter()
                .map(|_| Node::Identifier {
                    name: "$match_value".to_string(),
                    span,
                })
                .collect();
            let parameters = names
                .into_iter()
                .map(|name| ("any".to_string(), name))
                .collect();
            let mut stmts = Vec::new();
            if let Some(guard) = guard {
                stmts.push(Node::ExpressionStatement {
                    expr: Box::new(guard.clone()),
                    span,
                });
            }
            stmts.push(Node::ReturnStatement {
                value: Some(Box::new(body.clone())),
                span,
            });
            branches.push(Node::CallExpression {
                function: Box::new(Node::FunctionLiteral {
                    parameters,
                    body: Box::new(Node::Block { stmts, span }),
                    requires: Vec::new(),
                    ensures: Vec::new(),
                    recovers_to: None,
                    return_type: None,
                    span,
                    explicit_effect: None,
                }),
                arguments,
                span,
            });
        }
        // Both alternatives are joined by provenance analysis. No pattern or
        // guard feasibility is assumed, and each arm has its own lexical scope.
        let mut result = branches.pop().unwrap_or_else(|| scrutinee.as_ref().clone());
        for arm in branches.into_iter().rev() {
            result = Node::IfStatement {
                condition: Box::new(Node::BooleanLiteral { value: true, span }),
                consequence: Box::new(arm),
                alternative: Some(Box::new(result)),
                span,
            };
        }
        // A single outer parameter preserves scrutinee calls even for wildcards
        // without duplicating a large scrutinee once per pattern binding.
        result = Node::CallExpression {
            function: Box::new(Node::FunctionLiteral {
                parameters: vec![("any".to_string(), "$match_value".to_string())],
                body: Box::new(Node::Block {
                    stmts: vec![Node::ReturnStatement {
                        value: Some(Box::new(result)),
                        span,
                    }],
                    span,
                }),
                requires: Vec::new(),
                ensures: Vec::new(),
                recovers_to: None,
                return_type: None,
                span,
                explicit_effect: None,
            }),
            arguments: vec![scrutinee.as_ref().clone()],
            span,
        };
        *node = if exits {
            Node::ReturnStatement {
                value: Some(Box::new(result)),
                span,
            }
        } else {
            result
        };
    }
    let mut result = program.clone();
    rewrite(&mut result);
    result
}
