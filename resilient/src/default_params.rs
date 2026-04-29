//! RES-326: default parameter values — `fn foo(x: Int, y: Int = 0)`.
//!
//! Default values for trailing parameters allow call sites to omit them.
//! Resolution is a post-parse lowering pass over the AST: this module
//! walks the program, collects every top-level `fn` signature together
//! with its per-parameter default expressions, then rewrites every
//! `CallExpression` whose callee is a known top-level fn by appending
//! cloned defaults for any trailing arguments that were omitted.
//!
//! This keeps the hot interpreter path completely free of
//! default-parameter awareness — the call site, after lowering, looks
//! identical to a fully-explicit positional call.
//!
//! Resolution rules:
//! - Only trailing parameters may be omitted at a call site.
//! - If a call provides fewer arguments than the function has
//!   parameters, the missing trailing slots must all have defaults.
//! - Mixing positional omission with named args is handled by running
//!   `named_args::lower_program` first (which is the existing ordering
//!   in `crate::parse`).
//!
//! Accepted by `lower_program`; `check` is an MVP no-op.

use crate::Node;
use std::collections::HashMap;

/// A function's default-value information: parallel to `parameters`.
#[derive(Clone)]
struct FnDefaults {
    /// Number of parameters the function declares.
    param_count: usize,
    /// `defaults[i]` is the default expression for parameter `i`, or
    /// `None` when that parameter has no default.
    defaults: Vec<Option<Box<Node>>>,
}

/// Walk a parsed `Node::Program` and rewrite every `CallExpression`
/// whose callee is a known top-level fn by filling in any missing
/// trailing arguments with that fn's declared default expressions.
///
/// Runs after `named_args::lower_program` so the argument list is
/// already in positional order when we get here.
pub fn lower_program(program: &mut Node) {
    let mut sigs: HashMap<String, FnDefaults> = HashMap::new();
    collect_defaults(program, &mut sigs);
    rewrite_calls(program, &sigs);
}

/// RES-326: type-check pass for default parameter values.
/// MVP: all defaults are accepted — a future ticket may enforce that
/// defaults are constant-foldable or otherwise restricted.
pub fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

fn collect_defaults(node: &Node, sigs: &mut HashMap<String, FnDefaults>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                collect_defaults(&s.node, sigs);
            }
        }
        Node::Function {
            name,
            parameters,
            defaults,
            ..
        } => {
            sigs.insert(
                name.clone(),
                FnDefaults {
                    param_count: parameters.len(),
                    defaults: defaults.clone(),
                },
            );
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                if let Node::Function {
                    name,
                    parameters,
                    defaults,
                    ..
                } = m
                {
                    sigs.insert(
                        name.clone(),
                        FnDefaults {
                            param_count: parameters.len(),
                            defaults: defaults.clone(),
                        },
                    );
                }
            }
        }
        _ => {}
    }
}

/// Post-order rewrite: visit children before rewriting the call at each
/// node so nested calls have their defaults filled in first.
fn rewrite_calls(node: &mut Node, sigs: &HashMap<String, FnDefaults>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts.iter_mut() {
                rewrite_calls(&mut s.node, sigs);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts.iter_mut() {
                rewrite_calls(s, sigs);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            rewrite_calls(body, sigs);
            for r in requires.iter_mut() {
                rewrite_calls(r, sigs);
            }
            for e in ensures.iter_mut() {
                rewrite_calls(e, sigs);
            }
            if let Some(rec) = recovers_to {
                rewrite_calls(rec, sigs);
            }
        }
        Node::FunctionLiteral {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            rewrite_calls(body, sigs);
            for r in requires.iter_mut() {
                rewrite_calls(r, sigs);
            }
            for e in ensures.iter_mut() {
                rewrite_calls(e, sigs);
            }
            if let Some(rec) = recovers_to {
                rewrite_calls(rec, sigs);
            }
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods.iter_mut() {
                rewrite_calls(m, sigs);
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. }
        | Node::ExpressionStatement { expr: value, .. } => rewrite_calls(value, sigs),
        Node::ReturnStatement { value: Some(v), .. } => {
            rewrite_calls(v, sigs);
        }
        Node::ReturnStatement { value: None, .. } => {}
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            rewrite_calls(condition, sigs);
            rewrite_calls(consequence, sigs);
            if let Some(alt) = alternative {
                rewrite_calls(alt, sigs);
            }
        }
        Node::WhileStatement {
            condition,
            body,
            invariants,
            ..
        } => {
            rewrite_calls(condition, sigs);
            rewrite_calls(body, sigs);
            for inv in invariants.iter_mut() {
                rewrite_calls(inv, sigs);
            }
        }
        Node::ForInStatement {
            iterable,
            body,
            invariants,
            ..
        } => {
            rewrite_calls(iterable, sigs);
            rewrite_calls(body, sigs);
            for inv in invariants.iter_mut() {
                rewrite_calls(inv, sigs);
            }
        }
        Node::PrefixExpression { right, .. } => rewrite_calls(right, sigs),
        Node::InfixExpression { left, right, .. } => {
            rewrite_calls(left, sigs);
            rewrite_calls(right, sigs);
        }
        Node::IndexExpression { target, index, .. } => {
            rewrite_calls(target, sigs);
            rewrite_calls(index, sigs);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            rewrite_calls(target, sigs);
            rewrite_calls(index, sigs);
            rewrite_calls(value, sigs);
        }
        Node::FieldAccess { target, .. } => rewrite_calls(target, sigs),
        Node::FieldAssignment { target, value, .. } => {
            rewrite_calls(target, sigs);
            rewrite_calls(value, sigs);
        }
        Node::ArrayLiteral { items, .. } => {
            for it in items.iter_mut() {
                rewrite_calls(it, sigs);
            }
        }
        Node::MapLiteral { entries, .. } => {
            for (k, v) in entries.iter_mut() {
                rewrite_calls(k, sigs);
                rewrite_calls(v, sigs);
            }
        }
        Node::SetLiteral { items, .. } => {
            for it in items.iter_mut() {
                rewrite_calls(it, sigs);
            }
        }
        Node::TryExpression { expr, .. } => rewrite_calls(expr, sigs),
        Node::OptionalChain { object, access, .. } => {
            rewrite_calls(object, sigs);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args.iter_mut() {
                    rewrite_calls(a, sigs);
                }
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            rewrite_calls(scrutinee, sigs);
            for (_pat, guard, body) in arms.iter_mut() {
                if let Some(g) = guard {
                    rewrite_calls(g, sigs);
                }
                rewrite_calls(body, sigs);
            }
        }
        Node::TryCatch { body, handlers, .. } => {
            for s in body.iter_mut() {
                rewrite_calls(s, sigs);
            }
            for (_v, handler_body) in handlers.iter_mut() {
                for s in handler_body.iter_mut() {
                    rewrite_calls(s, sigs);
                }
            }
        }
        Node::Quantifier { range, body, .. } => {
            match range {
                crate::quantifiers::QuantRange::Range { lo, hi } => {
                    rewrite_calls(lo, sigs);
                    rewrite_calls(hi, sigs);
                }
                crate::quantifiers::QuantRange::Iterable(expr) => rewrite_calls(expr, sigs),
            }
            rewrite_calls(body, sigs);
        }
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            rewrite_calls(condition, sigs);
            if let Some(m) = message {
                rewrite_calls(m, sigs);
            }
        }
        Node::InvariantStatement { expr, .. } => rewrite_calls(expr, sigs),
        Node::LiveBlock {
            body, invariants, ..
        } => {
            rewrite_calls(body, sigs);
            for inv in invariants.iter_mut() {
                rewrite_calls(inv, sigs);
            }
        }
        Node::StructLiteral { fields, .. } => {
            for (_n, v) in fields.iter_mut() {
                rewrite_calls(v, sigs);
            }
        }
        // The hot path: if a call to a known top-level fn is missing
        // trailing arguments and those positions have defaults, fill
        // them in before the interpreter ever sees the call.
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            rewrite_calls(function, sigs);
            for a in arguments.iter_mut() {
                rewrite_calls(a, sigs);
            }
            let callee_name = if let Node::Identifier { name, .. } = function.as_ref() {
                Some(name.clone())
            } else {
                None
            };
            if let Some(name) = callee_name
                && let Some(sig) = sigs.get(&name)
            {
                let provided = arguments.len();
                let total = sig.param_count;
                if provided < total {
                    // Append cloned defaults for each missing trailing slot.
                    for i in provided..total {
                        if let Some(default_expr) = &sig.defaults.get(i).and_then(|d| d.as_ref()) {
                            arguments.push(*(*default_expr).clone());
                        }
                        // If there is no default for this slot, leave the
                        // argument list short — the runtime will surface a
                        // clean "missing argument" diagnostic.
                    }
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    fn int_lit(v: i64) -> Node {
        Node::IntegerLiteral {
            value: v,
            span: Span::default(),
        }
    }

    fn ident(name: &str) -> Node {
        Node::Identifier {
            name: name.to_string(),
            span: Span::default(),
        }
    }

    fn call(callee: &str, args: Vec<Node>) -> Node {
        Node::CallExpression {
            function: Box::new(ident(callee)),
            arguments: args,
            span: Span::default(),
        }
    }

    fn make_fn(name: &str, param_count: usize, defaults: Vec<Option<Box<Node>>>) -> Node {
        // Build a dummy parameter list of `(int, pN)` pairs.
        let parameters: Vec<(String, String)> = (0..param_count)
            .map(|i| ("int".to_string(), format!("p{}", i)))
            .collect();
        Node::Function {
            name: name.to_string(),
            parameters,
            defaults,
            body: Box::new(Node::Block {
                stmts: Vec::new(),
                span: Span::default(),
            }),
            requires: Vec::new(),
            ensures: Vec::new(),
            recovers_to: None,
            return_type: None,
            span: Span::default(),
            pure: false,
            effects: crate::EffectSet::io(),
            type_params: Vec::new(),
            type_param_bounds: Vec::new(),
            fails: Vec::new(),
        }
    }

    #[test]
    fn call_with_all_args_is_unchanged() {
        let fn_decl = make_fn("add", 2, vec![None, None]);
        let call_node = call("add", vec![int_lit(1), int_lit(2)]);
        let mut prog = Node::Program(vec![
            crate::span::Spanned {
                node: fn_decl,
                span: Span::default(),
            },
            crate::span::Spanned {
                node: call_node,
                span: Span::default(),
            },
        ]);
        lower_program(&mut prog);
        if let Node::Program(stmts) = &prog {
            if let Node::CallExpression { arguments, .. } = &stmts[1].node {
                assert_eq!(arguments.len(), 2, "should still have 2 args");
            } else {
                panic!("expected CallExpression");
            }
        }
    }

    #[test]
    fn missing_arg_with_default_is_filled_in() {
        // fn f(p0: int, p1: int = 42)
        let fn_decl = make_fn("f", 2, vec![None, Some(Box::new(int_lit(42)))]);
        let call_node = call("f", vec![int_lit(1)]);
        let mut prog = Node::Program(vec![
            crate::span::Spanned {
                node: fn_decl,
                span: Span::default(),
            },
            crate::span::Spanned {
                node: call_node,
                span: Span::default(),
            },
        ]);
        lower_program(&mut prog);
        if let Node::Program(stmts) = &prog {
            if let Node::CallExpression { arguments, .. } = &stmts[1].node {
                assert_eq!(arguments.len(), 2, "default should have been inserted");
                match &arguments[1] {
                    Node::IntegerLiteral { value, .. } => assert_eq!(*value, 42),
                    other => panic!("expected IntegerLiteral(42), got {:?}", other),
                }
            } else {
                panic!("expected CallExpression");
            }
        }
    }

    #[test]
    fn two_missing_defaults_both_filled_in() {
        // fn g(p0: int, p1: int = 10, p2: int = 20)
        let fn_decl = make_fn(
            "g",
            3,
            vec![
                None,
                Some(Box::new(int_lit(10))),
                Some(Box::new(int_lit(20))),
            ],
        );
        let call_node = call("g", vec![int_lit(1)]);
        let mut prog = Node::Program(vec![
            crate::span::Spanned {
                node: fn_decl,
                span: Span::default(),
            },
            crate::span::Spanned {
                node: call_node,
                span: Span::default(),
            },
        ]);
        lower_program(&mut prog);
        if let Node::Program(stmts) = &prog
            && let Node::CallExpression { arguments, .. } = &stmts[1].node
        {
            assert_eq!(arguments.len(), 3);
            match &arguments[1] {
                Node::IntegerLiteral { value, .. } => assert_eq!(*value, 10),
                other => panic!("expected 10, got {:?}", other),
            }
            match &arguments[2] {
                Node::IntegerLiteral { value, .. } => assert_eq!(*value, 20),
                other => panic!("expected 20, got {:?}", other),
            }
        }
    }

    #[test]
    fn unknown_callee_call_is_left_unchanged() {
        let call_node = call("unknown", vec![int_lit(5)]);
        let mut prog = Node::Program(vec![crate::span::Spanned {
            node: call_node,
            span: Span::default(),
        }]);
        lower_program(&mut prog);
        if let Node::Program(stmts) = &prog
            && let Node::CallExpression { arguments, .. } = &stmts[0].node
        {
            assert_eq!(arguments.len(), 1);
        }
    }
}
