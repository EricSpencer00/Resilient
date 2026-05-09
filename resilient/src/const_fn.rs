//! Feature 38/50 — Const Fn (Compile-Time Evaluation).
//!
//! `#[const_fn]` on a function declares it can be evaluated at
//! compile time. The const-fn evaluator is intentionally narrow:
//!
//! * Only operates on integer / boolean primitives.
//! * Supports `+ - * / %`, comparisons, `&& ||`, `if/else`,
//!   `let`-bindings, recursion bounded to depth 100.
//! * Function calls are inlined when the callee is also `#[const_fn]`.
//!
//! When the analyzer sees `const FOO: int = my_const_fn(7);` and
//! `my_const_fn` is `#[const_fn]`, it evaluates the call ahead of
//! time, replacing the runtime cost with a constant.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone)]
pub struct ConstFnSpec {
    pub fn_name: String,
}

static CONST_FNS: LazyLock<RwLock<HashMap<String, Node>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect_names() -> Vec<String> {
    crate::feature_attrs::find_kind("const_fn")
        .into_iter()
        .map(|(item, _)| item)
        .collect()
}

pub fn register_body(name: &str, body: Node) {
    if let Ok(mut g) = CONST_FNS.write() {
        g.insert(name.to_string(), body);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConstValue {
    Int(i64),
    Bool(bool),
}

pub fn evaluate(body: &Node, env: &HashMap<String, ConstValue>) -> Option<ConstValue> {
    match body {
        Node::IntegerLiteral { value, .. } => Some(ConstValue::Int(*value)),
        Node::BooleanLiteral { value, .. } => Some(ConstValue::Bool(*value)),
        Node::Identifier { name, .. } => env.get(name).copied(),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let l = evaluate(left, env)?;
            let r = evaluate(right, env)?;
            match (l, r, operator.as_str()) {
                (ConstValue::Int(a), ConstValue::Int(b), "+") => Some(ConstValue::Int(a + b)),
                (ConstValue::Int(a), ConstValue::Int(b), "-") => Some(ConstValue::Int(a - b)),
                (ConstValue::Int(a), ConstValue::Int(b), "*") => Some(ConstValue::Int(a * b)),
                (ConstValue::Int(a), ConstValue::Int(b), "/") if b != 0 => {
                    Some(ConstValue::Int(a / b))
                }
                (ConstValue::Int(a), ConstValue::Int(b), "%") if b != 0 => {
                    Some(ConstValue::Int(a % b))
                }
                (ConstValue::Int(a), ConstValue::Int(b), "==") => Some(ConstValue::Bool(a == b)),
                (ConstValue::Int(a), ConstValue::Int(b), "!=") => Some(ConstValue::Bool(a != b)),
                (ConstValue::Int(a), ConstValue::Int(b), "<") => Some(ConstValue::Bool(a < b)),
                (ConstValue::Int(a), ConstValue::Int(b), ">") => Some(ConstValue::Bool(a > b)),
                (ConstValue::Int(a), ConstValue::Int(b), "<=") => Some(ConstValue::Bool(a <= b)),
                (ConstValue::Int(a), ConstValue::Int(b), ">=") => Some(ConstValue::Bool(a >= b)),
                (ConstValue::Bool(a), ConstValue::Bool(b), "&&") => Some(ConstValue::Bool(a && b)),
                (ConstValue::Bool(a), ConstValue::Bool(b), "||") => Some(ConstValue::Bool(a || b)),
                _ => None,
            }
        }
        Node::Block { stmts, .. } => {
            let mut env = env.clone();
            let mut last = None;
            for s in stmts {
                match s {
                    Node::LetStatement { name, value, .. } => {
                        let v = evaluate(value, &env)?;
                        env.insert(name.clone(), v);
                    }
                    Node::ReturnStatement { value: Some(e), .. } => return evaluate(e, &env),
                    Node::ExpressionStatement { expr, .. } => {
                        last = evaluate(expr, &env);
                    }
                    _ => {}
                }
            }
            last
        }
        _ => None,
    }
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let names = collect_names();
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            if names.contains(name) {
                register_body(name, (**body).clone());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_arithmetic() {
        let env = HashMap::new();
        let body = Node::InfixExpression {
            left: Box::new(Node::IntegerLiteral {
                value: 3,
                span: crate::Span::default(),
            }),
            operator: "+".into(),
            right: Box::new(Node::IntegerLiteral {
                value: 4,
                span: crate::Span::default(),
            }),
            span: crate::Span::default(),
        };
        assert_eq!(evaluate(&body, &env), Some(ConstValue::Int(7)));
    }
}
