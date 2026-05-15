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
    eval_depth(body, env, 0)
}

fn eval_depth(body: &Node, env: &HashMap<String, ConstValue>, depth: u32) -> Option<ConstValue> {
    if depth > 100 {
        return None;
    }
    match body {
        Node::IntegerLiteral { value, .. } => Some(ConstValue::Int(*value)),
        Node::BooleanLiteral { value, .. } => Some(ConstValue::Bool(*value)),
        Node::Identifier { name, .. } => env.get(name).copied(),

        Node::PrefixExpression {
            operator, right, ..
        } => {
            let v = eval_depth(right, env, depth + 1)?;
            match (operator.as_str(), v) {
                ("-", ConstValue::Int(n)) => Some(ConstValue::Int(-n)),
                ("!", ConstValue::Bool(b)) => Some(ConstValue::Bool(!b)),
                _ => None,
            }
        }

        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            // Short-circuit `&&` and `||` before evaluating both sides.
            match operator.as_str() {
                "&&" => {
                    let lv = eval_depth(left, env, depth + 1)?;
                    if let ConstValue::Bool(false) = lv {
                        return Some(ConstValue::Bool(false));
                    }
                    let rv = eval_depth(right, env, depth + 1)?;
                    return match rv {
                        ConstValue::Bool(b) => Some(ConstValue::Bool(b)),
                        _ => None,
                    };
                }
                "||" => {
                    let lv = eval_depth(left, env, depth + 1)?;
                    if let ConstValue::Bool(true) = lv {
                        return Some(ConstValue::Bool(true));
                    }
                    let rv = eval_depth(right, env, depth + 1)?;
                    return match rv {
                        ConstValue::Bool(b) => Some(ConstValue::Bool(b)),
                        _ => None,
                    };
                }
                _ => {}
            }
            let l = eval_depth(left, env, depth + 1)?;
            let r = eval_depth(right, env, depth + 1)?;
            match (l, r, operator.as_str()) {
                (ConstValue::Int(a), ConstValue::Int(b), "+") => {
                    Some(ConstValue::Int(a.wrapping_add(b)))
                }
                (ConstValue::Int(a), ConstValue::Int(b), "-") => {
                    Some(ConstValue::Int(a.wrapping_sub(b)))
                }
                (ConstValue::Int(a), ConstValue::Int(b), "*") => {
                    Some(ConstValue::Int(a.wrapping_mul(b)))
                }
                (ConstValue::Int(a), ConstValue::Int(b), "/") if b != 0 => {
                    Some(ConstValue::Int(a / b))
                }
                (ConstValue::Int(a), ConstValue::Int(b), "%") if b != 0 => {
                    Some(ConstValue::Int(a % b))
                }
                (ConstValue::Int(a), ConstValue::Int(b), "&") => Some(ConstValue::Int(a & b)),
                (ConstValue::Int(a), ConstValue::Int(b), "|") => Some(ConstValue::Int(a | b)),
                (ConstValue::Int(a), ConstValue::Int(b), "^") => Some(ConstValue::Int(a ^ b)),
                (ConstValue::Int(a), ConstValue::Int(b), "<<") if (0..64).contains(&b) => {
                    Some(ConstValue::Int(a << b))
                }
                (ConstValue::Int(a), ConstValue::Int(b), ">>") if (0..64).contains(&b) => {
                    Some(ConstValue::Int(a >> b))
                }
                (ConstValue::Int(a), ConstValue::Int(b), "==") => Some(ConstValue::Bool(a == b)),
                (ConstValue::Int(a), ConstValue::Int(b), "!=") => Some(ConstValue::Bool(a != b)),
                (ConstValue::Int(a), ConstValue::Int(b), "<") => Some(ConstValue::Bool(a < b)),
                (ConstValue::Int(a), ConstValue::Int(b), ">") => Some(ConstValue::Bool(a > b)),
                (ConstValue::Int(a), ConstValue::Int(b), "<=") => Some(ConstValue::Bool(a <= b)),
                (ConstValue::Int(a), ConstValue::Int(b), ">=") => Some(ConstValue::Bool(a >= b)),
                (ConstValue::Bool(a), ConstValue::Bool(b), "==") => Some(ConstValue::Bool(a == b)),
                (ConstValue::Bool(a), ConstValue::Bool(b), "!=") => Some(ConstValue::Bool(a != b)),
                _ => None,
            }
        }

        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            let cond = eval_depth(condition, env, depth + 1)?;
            match cond {
                ConstValue::Bool(true) => eval_depth(consequence, env, depth + 1),
                ConstValue::Bool(false) => alternative
                    .as_ref()
                    .and_then(|a| eval_depth(a, env, depth + 1)),
                _ => None,
            }
        }

        Node::Block { stmts, .. } => {
            let mut env = env.clone();
            let mut last = None;
            for s in stmts {
                match s {
                    Node::LetStatement { name, value, .. } => {
                        let v = eval_depth(value, &env, depth + 1)?;
                        env.insert(name.clone(), v);
                    }
                    Node::ReturnStatement { value: Some(e), .. } => {
                        return eval_depth(e, &env, depth + 1);
                    }
                    Node::ExpressionStatement { expr, .. } => {
                        last = eval_depth(expr, &env, depth + 1);
                    }
                    Node::IfStatement { .. } => {
                        last = eval_depth(s, &env, depth + 1);
                    }
                    _ => {}
                }
            }
            last
        }

        // Inline registered const-fn calls.
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if let Ok(g) = CONST_FNS.read() {
                    if let Some(body_node) = g.get(name.as_str()) {
                        let body_clone = body_node.clone();
                        drop(g);
                        // Build env from argument values.
                        // We need parameter names — they're encoded in the registered body.
                        // Without them, evaluate the body with the current env extended by
                        // positional args as "__arg0", "__arg1", etc.
                        let mut call_env = env.clone();
                        for (i, arg) in arguments.iter().enumerate() {
                            if let Some(v) = eval_depth(arg, env, depth + 1) {
                                call_env.insert(format!("__arg{i}"), v);
                            }
                        }
                        return eval_depth(&body_clone, &call_env, depth + 1);
                    }
                }
            }
            None
        }

        _ => None,
    }
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let names = collect_names();
    // RES-1238: fast-reject. The diagnostic / body-registration only
    // fires for functions annotated `#[const_fn]`. When no such
    // attribute exists in the program (the overwhelming common case),
    // `names` is empty, every `names.contains(name)` call returns
    // false, and the per-statement loop produces no output. Skip the
    // loop entirely. Same shape as RES-1236 (`crash_only_cert`).
    if names.is_empty() {
        return Ok(());
    }
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

    fn int_lit(v: i64) -> Node {
        Node::IntegerLiteral {
            value: v,
            span: crate::Span::default(),
        }
    }

    fn bool_lit(v: bool) -> Node {
        Node::BooleanLiteral {
            value: v,
            span: crate::Span::default(),
        }
    }

    fn infix(left: Node, op: &str, right: Node) -> Node {
        Node::InfixExpression {
            left: Box::new(left),
            operator: op.into(),
            right: Box::new(right),
            span: crate::Span::default(),
        }
    }

    #[test]
    fn evaluates_arithmetic() {
        let env = HashMap::new();
        let body = infix(int_lit(3), "+", int_lit(4));
        assert_eq!(evaluate(&body, &env), Some(ConstValue::Int(7)));
    }

    #[test]
    fn evaluates_subtraction_and_multiply() {
        let env = HashMap::new();
        assert_eq!(
            evaluate(&infix(int_lit(10), "-", int_lit(3)), &env),
            Some(ConstValue::Int(7))
        );
        assert_eq!(
            evaluate(&infix(int_lit(6), "*", int_lit(7)), &env),
            Some(ConstValue::Int(42))
        );
    }

    #[test]
    fn evaluates_comparison_operators() {
        let env = HashMap::new();
        assert_eq!(
            evaluate(&infix(int_lit(3), "<", int_lit(5)), &env),
            Some(ConstValue::Bool(true))
        );
        assert_eq!(
            evaluate(&infix(int_lit(5), "==", int_lit(5)), &env),
            Some(ConstValue::Bool(true))
        );
        assert_eq!(
            evaluate(&infix(int_lit(3), ">", int_lit(5)), &env),
            Some(ConstValue::Bool(false))
        );
    }

    #[test]
    fn evaluates_boolean_operators() {
        let env = HashMap::new();
        assert_eq!(
            evaluate(&infix(bool_lit(true), "&&", bool_lit(false)), &env),
            Some(ConstValue::Bool(false))
        );
        assert_eq!(
            evaluate(&infix(bool_lit(true), "||", bool_lit(false)), &env),
            Some(ConstValue::Bool(true))
        );
    }

    #[test]
    fn divide_by_zero_returns_none() {
        let env = HashMap::new();
        assert_eq!(evaluate(&infix(int_lit(5), "/", int_lit(0)), &env), None);
    }

    #[test]
    fn identifier_lookup_uses_env() {
        let mut env = HashMap::new();
        env.insert("LIMIT".to_string(), ConstValue::Int(100));
        let node = Node::Identifier {
            name: "LIMIT".to_string(),
            span: crate::Span::default(),
        };
        assert_eq!(evaluate(&node, &env), Some(ConstValue::Int(100)));
    }

    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn evaluates_prefix_negation() {
        let env = HashMap::new();
        let neg = Node::PrefixExpression {
            operator: "-".into(),
            right: Box::new(int_lit(5)),
            span: crate::Span::default(),
        };
        assert_eq!(evaluate(&neg, &env), Some(ConstValue::Int(-5)));
    }

    #[test]
    fn evaluates_prefix_not() {
        let env = HashMap::new();
        let not = Node::PrefixExpression {
            operator: "!".into(),
            right: Box::new(bool_lit(true)),
            span: crate::Span::default(),
        };
        assert_eq!(evaluate(&not, &env), Some(ConstValue::Bool(false)));
    }

    #[test]
    fn evaluates_if_true_branch() {
        let env = HashMap::new();
        let cond = bool_lit(true);
        let then_block = Node::Block {
            stmts: vec![Node::ReturnStatement {
                value: Some(Box::new(int_lit(42))),
                span: crate::Span::default(),
            }],
            span: crate::Span::default(),
        };
        let else_block = Node::Block {
            stmts: vec![Node::ReturnStatement {
                value: Some(Box::new(int_lit(0))),
                span: crate::Span::default(),
            }],
            span: crate::Span::default(),
        };
        let if_node = Node::IfStatement {
            condition: Box::new(cond),
            consequence: Box::new(then_block),
            alternative: Some(Box::new(else_block)),
            span: crate::Span::default(),
        };
        assert_eq!(evaluate(&if_node, &env), Some(ConstValue::Int(42)));
    }

    #[test]
    fn evaluates_if_false_branch() {
        let env = HashMap::new();
        let cond = bool_lit(false);
        let then_block = Node::Block {
            stmts: vec![Node::ReturnStatement {
                value: Some(Box::new(int_lit(1))),
                span: crate::Span::default(),
            }],
            span: crate::Span::default(),
        };
        let else_block = Node::Block {
            stmts: vec![Node::ReturnStatement {
                value: Some(Box::new(int_lit(99))),
                span: crate::Span::default(),
            }],
            span: crate::Span::default(),
        };
        let if_node = Node::IfStatement {
            condition: Box::new(cond),
            consequence: Box::new(then_block),
            alternative: Some(Box::new(else_block)),
            span: crate::Span::default(),
        };
        assert_eq!(evaluate(&if_node, &env), Some(ConstValue::Int(99)));
    }

    #[test]
    fn evaluates_bitwise_and() {
        let env = HashMap::new();
        assert_eq!(
            evaluate(&infix(int_lit(0b1100), "&", int_lit(0b1010)), &env),
            Some(ConstValue::Int(0b1000))
        );
    }

    #[test]
    fn evaluates_bitwise_or() {
        let env = HashMap::new();
        assert_eq!(
            evaluate(&infix(int_lit(0b1100), "|", int_lit(0b1010)), &env),
            Some(ConstValue::Int(0b1110))
        );
    }

    #[test]
    fn evaluates_shift_left() {
        let env = HashMap::new();
        assert_eq!(
            evaluate(&infix(int_lit(1), "<<", int_lit(4)), &env),
            Some(ConstValue::Int(16))
        );
    }

    #[test]
    fn short_circuit_and_false() {
        // `false && <unevaluable>` should return false without evaluating RHS.
        let env = HashMap::new();
        let unevaluable = Node::Identifier {
            name: "undefined_var".into(),
            span: crate::Span::default(),
        };
        let expr = Node::InfixExpression {
            left: Box::new(bool_lit(false)),
            operator: "&&".into(),
            right: Box::new(unevaluable),
            span: crate::Span::default(),
        };
        assert_eq!(evaluate(&expr, &env), Some(ConstValue::Bool(false)));
    }

    #[test]
    fn short_circuit_or_true() {
        // `true || <unevaluable>` should return true without evaluating RHS.
        let env = HashMap::new();
        let unevaluable = Node::Identifier {
            name: "undefined_var".into(),
            span: crate::Span::default(),
        };
        let expr = Node::InfixExpression {
            left: Box::new(bool_lit(true)),
            operator: "||".into(),
            right: Box::new(unevaluable),
            span: crate::Span::default(),
        };
        assert_eq!(evaluate(&expr, &env), Some(ConstValue::Bool(true)));
    }
}
