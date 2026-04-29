// RES-360: Closed-form invariant checker for live{} recovery bodies.
//
// V2 TLA+ verification requires recovery expressions to be closed-form
// (encodable as state transitions). This pass detects violations early:
// - Opaque FFI calls (malloc, strlen, etc.)
// - Direct recursion
// - Closures capturing from outer scope
//
// V1 emits warnings; V2 will escalate to errors under --v2-strict.

use crate::Node;
use std::collections::HashSet;

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let mut ctx = Context::new();
    ctx.collect_declarations(program);
    ctx.check_live_blocks(program);
    Ok(())
}

struct Context {
    extern_fns: HashSet<String>,
    current_fn: Option<String>,
}

impl Context {
    fn new() -> Self {
        Context {
            extern_fns: HashSet::new(),
            current_fn: None,
        }
    }

    fn collect_declarations(&mut self, program: &Node) {
        let Node::Program(statements) = program else {
            return;
        };
        for stmt in statements {
            if let Node::Extern { decls, .. } = &stmt.node {
                for decl in decls {
                    self.extern_fns.insert(decl.resilient_name.clone());
                }
            }
        }
    }

    fn check_live_blocks(&mut self, program: &Node) {
        let Node::Program(statements) = program else {
            return;
        };
        for stmt in statements {
            self.walk_for_live_blocks(&stmt.node);
        }
    }

    fn walk_for_live_blocks(&mut self, node: &Node) {
        match node {
            Node::Function { name, body, .. } => {
                let prev_fn = self.current_fn.take();
                self.current_fn = Some(name.clone());
                self.walk_for_live_blocks(body);
                self.current_fn = prev_fn;
            }
            Node::LiveBlock {
                body, invariants, ..
            } => {
                self.check_node(body);
                for inv in invariants {
                    self.check_node(inv);
                }
            }
            Node::Block { stmts, .. } => {
                for stmt in stmts {
                    self.walk_for_live_blocks(stmt);
                }
            }
            Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
                self.walk_for_live_blocks(condition);
                self.walk_for_live_blocks(consequence);
                if let Some(alt) = alternative {
                    self.walk_for_live_blocks(alt);
                }
            }
            Node::WhileStatement {
                condition, body, ..
            } => {
                self.walk_for_live_blocks(condition);
                self.walk_for_live_blocks(body);
            }
            Node::ForInStatement { iterable, body, .. } => {
                self.walk_for_live_blocks(iterable);
                self.walk_for_live_blocks(body);
            }
            _ => {}
        }
    }

    fn check_node(&mut self, node: &Node) {
        match node {
            Node::CallExpression {
                function,
                arguments,
                ..
            } => {
                if let Some(fn_name) = self.extract_identifier(function) {
                    if self.extern_fns.contains(&fn_name) {
                        eprintln!(
                            "warning: opaque FFI call to '{}' in recovery body \
                            cannot be modeled as TLA+ action",
                            fn_name
                        );
                    } else if let Some(ref current) = self.current_fn {
                        if &fn_name == current {
                            eprintln!(
                                "warning: function '{}' recursively calls itself \
                                in recovery body",
                                current
                            );
                        }
                    }
                }
                for arg in arguments {
                    self.check_node(arg);
                }
            }
            Node::FunctionLiteral { .. } => {
                let free = crate::free_vars::free_vars(node);
                if !free.is_empty() {
                    let captured: Vec<_> = free.iter().cloned().collect();
                    eprintln!(
                        "warning: closure in recovery body captures [{}] from outer scope",
                        captured.join(", ")
                    );
                }
            }
            Node::Block { stmts, .. } => {
                for stmt in stmts {
                    self.check_node(stmt);
                }
            }
            Node::LetStatement { value, .. } => {
                self.check_node(value);
            }
            Node::Assignment { value, .. } => {
                self.check_node(value);
            }
            Node::Assert { condition, .. } => {
                self.check_node(condition);
            }
            Node::Assume { condition, .. } => {
                self.check_node(condition);
            }
            Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
                self.check_node(condition);
                self.check_node(consequence);
                if let Some(alt) = alternative {
                    self.check_node(alt);
                }
            }
            Node::WhileStatement {
                condition, body, ..
            } => {
                self.check_node(condition);
                self.check_node(body);
            }
            Node::ForInStatement { iterable, body, .. } => {
                self.check_node(iterable);
                self.check_node(body);
            }
            Node::InfixExpression { left, right, .. } => {
                self.check_node(left);
                self.check_node(right);
            }
            Node::PrefixExpression { right, .. } => {
                self.check_node(right);
            }
            Node::IndexExpression { target, index, .. } => {
                self.check_node(target);
                self.check_node(index);
            }
            Node::FieldAccess { target, .. } => {
                self.check_node(target);
            }
            Node::ArrayLiteral { items, .. } => {
                for elem in items {
                    self.check_node(elem);
                }
            }
            Node::ExpressionStatement { expr, .. } => {
                self.check_node(expr);
            }
            _ => {}
        }
    }

    fn extract_identifier(&self, node: &Node) -> Option<String> {
        match node {
            Node::Identifier { name, .. } => Some(name.clone()),
            _ => None,
        }
    }
}
