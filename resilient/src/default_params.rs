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

// RES-1615: `check` is no longer called from `EXTENSION_PASSES`
// (the body is `Ok(())`; the real default-arg rewrite happens via
// `collect_defaults` + `rewrite_calls` from a different path).
// The module-level `dead_code` allow matches the pattern used in
// `causal_trace.rs`, `package_manager.rs`, `mutation_testing.rs`,
// etc. when those passes were dropped from the extension-passes
// fan-out.
#![allow(dead_code)]

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
///
/// RES-1311: fast-reject. The `rewrite_calls` walk only does work
/// when a CallExpression's callee is a known fn AND one of its
/// trailing parameters has a declared default. For every program
/// where no function declares any default — the overwhelming
/// majority of `examples/` and the test suite — the rewrite walk
/// is pure overhead. Pre-scan the collected sigs: if no
/// `FnDefaults` entry has a `Some(_)` slot, skip the walk entirely.
/// `crate::newtypes::lower_program` follows the same shape.
pub fn lower_program(program: &mut Node) {
    // RES-1800: pre-size to 8 — `collect_defaults` only inserts fns
    // that declare at least one default. Programs using defaults
    // typically have a handful; 8 covers the common case without
    // rehash churn. Same shape as RES-1794's `named_args::lower_program`.
    let mut sigs: HashMap<String, FnDefaults> = HashMap::with_capacity(8);
    collect_defaults(program, &mut sigs);
    // RES-1475: `collect_defaults` now only inserts functions that
    // have at least one declared default, so `sigs.is_empty()`
    // implies no default anywhere. The previous shape iterated every
    // entry to re-check `defaults.iter().any(|d| d.is_some())` even
    // though the same check is now the insertion gate.
    if sigs.is_empty() {
        return;
    }
    rewrite_calls(program, &sigs);
}

/// RES-326: type-check pass for default parameter values.
///
/// Validates two invariants across every function declaration:
///
/// 1. **Trailing-only defaults** — defaults may only appear on
///    trailing parameters.  A function like `fn f(int x = 0, int y)`
///    is rejected because `y` comes after the defaulted `x` with no
///    default of its own, which makes call-site resolution ambiguous.
///
/// 2. **Constant defaults** — default expressions must be compile-time
///    constants (integer, float, string, bool literals, or `null`/
///    `None` identifiers). Dynamic defaults (function calls, arithmetic,
///    variable references) are rejected: they would be evaluated at
///    parse time in `lower_program`, producing unexpected behaviour.
pub fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Ok(()),
    };
    for s in stmts {
        check_fn_defaults(&s.node, source_path)?;
    }
    Ok(())
}

fn check_fn_defaults(node: &Node, source_path: &str) -> Result<(), String> {
    match node {
        Node::Function {
            name,
            parameters,
            defaults,
            span,
            ..
        } => {
            check_defaults_for_fn(name, parameters, defaults, span, source_path)?;
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                if let Node::Function {
                    name,
                    parameters,
                    defaults,
                    span,
                    ..
                } = m
                {
                    check_defaults_for_fn(name, parameters, defaults, span, source_path)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn check_defaults_for_fn(
    fn_name: &str,
    parameters: &[(String, String)],
    defaults: &[Option<Box<Node>>],
    span: &crate::Span,
    source_path: &str,
) -> Result<(), String> {
    let loc = if span.start.line > 0 {
        format!(
            "{}:{}:{}: ",
            source_path, span.start.line, span.start.column
        )
    } else {
        format!("{}: ", source_path)
    };

    // Rule 1: defaults must be trailing — once a gap (None after Some)
    // is found, the declaration is ambiguous.
    let mut saw_default = false;
    for (i, ((_ty, pname), default)) in parameters.iter().zip(defaults.iter()).enumerate() {
        if default.is_some() {
            saw_default = true;
        } else if saw_default {
            return Err(format!(
                "{loc}fn `{fn_name}`: parameter `{pname}` (position {i}) \
                 has no default but follows a defaulted parameter — \
                 defaults must be trailing"
            ));
        }
    }

    // Rule 2: default expressions must be compile-time constants.
    for ((_ty, pname), default) in parameters.iter().zip(defaults.iter()) {
        let Some(expr) = default else { continue };
        if !is_const_default(expr) {
            return Err(format!(
                "{loc}fn `{fn_name}`: default for parameter `{pname}` must \
                 be a compile-time constant (integer, float, string, bool, \
                 or `null`/`none`)"
            ));
        }
    }
    Ok(())
}

/// Returns true when the expression is acceptable as a default value:
/// a literal or a well-known constant identifier.
fn is_const_default(node: &Node) -> bool {
    matches!(
        node,
        Node::IntegerLiteral { .. }
            | Node::FloatLiteral { .. }
            | Node::StringLiteral { .. }
            | Node::BooleanLiteral { .. }
    ) || matches!(
        node,
        Node::Identifier { name, .. } if name == "null" || name == "none" || name == "None"
    )
}

fn collect_defaults(node: &Node, sigs: &mut HashMap<String, FnDefaults>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                collect_defaults(&s.node, sigs);
            }
        }
        // RES-1475: skip insertion for functions whose defaults
        // slice is entirely None. The downstream `rewrite_calls`
        // only fills in MISSING trailing args from declared
        // defaults; for a function with no Some(_) slot, no
        // rewrite would ever fire even if its name is in `sigs`.
        // The previous shape cloned the full
        // `Vec<Option<Box<Node>>>` per Function — for programs
        // where no function declares any default (the overwhelming
        // majority), the entire `sigs` HashMap was populated only
        // to be discarded at the `any_default` check in
        // `lower_program`. Use a match guard so newer clippy's
        // `collapsible_match` is happy.
        Node::Function {
            name,
            parameters,
            defaults,
            ..
        } if defaults.iter().any(|d| d.is_some()) => {
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
                    && defaults.iter().any(|d| d.is_some())
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
        //
        // RES-2042: borrow the callee name as `&str` instead of cloning.
        // `HashMap<String, V>::get` accepts `&str` via `String: Borrow<str>`,
        // so the lookup works without an owned conversion. The previous
        // shape cloned `name` on every CallExpression — pure overhead
        // for programs where no callee has declared defaults.
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            rewrite_calls(function, sigs);
            for a in arguments.iter_mut() {
                rewrite_calls(a, sigs);
            }
            let callee_name: Option<&str> = if let Node::Identifier { name, .. } = function.as_ref()
            {
                Some(name.as_str())
            } else {
                None
            };
            if let Some(name) = callee_name
                && let Some(sig) = sigs.get(name)
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
            is_pub: false,
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

    // ---- check() tests ----

    fn wrap(node: Node) -> Node {
        Node::Program(vec![crate::span::Spanned {
            node,
            span: Span::default(),
        }])
    }

    #[test]
    fn check_trailing_defaults_ok() {
        // fn f(int p0, int p1 = 0) — p1 is trailing → OK
        let f = make_fn("f", 2, vec![None, Some(Box::new(int_lit(0)))]);
        let prog = wrap(f);
        assert!(check(&prog, "test").is_ok(), "trailing default should pass");
    }

    #[test]
    fn check_non_trailing_default_errors() {
        // fn f(int p0 = 0, int p1) — p1 has no default but follows p0 → error
        let f = make_fn("f", 2, vec![Some(Box::new(int_lit(0))), None]);
        let prog = wrap(f);
        let err = check(&prog, "test");
        assert!(err.is_err(), "non-trailing default must be rejected");
        let msg = err.unwrap_err();
        assert!(
            msg.contains("trailing"),
            "error must mention trailing: {msg}"
        );
    }

    #[test]
    fn check_non_const_default_errors() {
        // fn f(int p0 = some_var) — some_var is not a constant → error
        let f = make_fn("f", 1, vec![Some(Box::new(ident("some_var")))]);
        let prog = wrap(f);
        let err = check(&prog, "test");
        assert!(err.is_err(), "non-const default must be rejected");
        let msg = err.unwrap_err();
        assert!(
            msg.contains("compile-time constant"),
            "error must mention constant: {msg}"
        );
    }

    #[test]
    fn check_null_default_ok() {
        // fn f(int p0 = null) — null is an accepted constant
        let f = make_fn("f", 1, vec![Some(Box::new(ident("null")))]);
        let prog = wrap(f);
        assert!(check(&prog, "test").is_ok(), "null default should pass");
    }

    #[test]
    fn check_no_defaults_ok() {
        // fn f(int p0, int p1) — no defaults → check is a no-op
        let f = make_fn("f", 2, vec![None, None]);
        let prog = wrap(f);
        assert!(check(&prog, "test").is_ok(), "no-defaults should pass");
    }
}
