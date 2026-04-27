//! RES-325: named function arguments — `foo(x: 1, y: 2)`.
//!
//! Resolution happens as a single post-parse pass over the program:
//! the parser leaves any `name: expr` pair as a `Node::NamedArg`
//! node inside a call's `arguments` vector, and [`lower_program`]
//! walks the program once, collects every top-level fn signature,
//! and rewrites each call whose argument list mentions named args
//! into a positional-only list. This keeps the hot interpreter
//! path completely free of named-arg awareness — recursive
//! programs like `fib` are very sensitive to per-call frame size
//! in debug builds, so any `if has_named(...)` check inside the
//! `eval` CallExpression arm is enough to push them past the
//! 2 MiB default test-thread stack.
//!
//! Resolution rules:
//!
//! - Positional args occupy the first N parameter slots in order.
//! - Each named arg targets the parameter whose name matches its
//!   label, regardless of textual position.
//! - It is an error if a named arg's label does not match any
//!   parameter name (with a "did you mean?" suggestion).
//! - It is an error if the same parameter is targeted by both a
//!   positional and a named arg.
//! - It is an error if two named args share the same label.
//! - It is an error if a parameter ends up unbound after resolution.
//!
//! The lowering is best-effort: calls whose callee is not a
//! statically-resolvable top-level fn (closures, builtins,
//! method-receiver dispatch) keep their `Node::NamedArg` nodes
//! and surface a runtime error if they ever execute. Acceptance
//! criteria for RES-325 only require named args on top-level fns.

use crate::Node;
use crate::did_you_mean::suggest;
use std::collections::HashMap;

/// Reorder `arguments` (which may contain `Node::NamedArg` entries)
/// into a flat positional vector matching `param_names`. Returns
/// `Err(diagnostic)` on any of the resolution-rule violations
/// described at the module level.
pub fn resolve(
    callee_label: &str,
    param_names: &[String],
    arguments: &[Node],
) -> Result<Vec<Node>, String> {
    // If no named args are present we can hand the original list
    // back unchanged — preserves identity and avoids any clones for
    // the existing positional-only call path.
    if !arguments.iter().any(|n| matches!(n, Node::NamedArg { .. })) {
        return Ok(arguments.to_vec());
    }

    // Pre-allocate one slot per parameter. `None` means the slot has
    // not yet been bound; a duplicate write is a diagnostic.
    let mut slots: Vec<Option<Node>> = vec![None; param_names.len()];

    // Walk arguments left-to-right. Positional args fill the next
    // available slot; named args resolve to the slot of the parameter
    // they label.
    let mut next_pos = 0usize;
    for arg in arguments {
        match arg {
            Node::NamedArg { name, value, .. } => {
                let Some(idx) = param_names.iter().position(|p| p == name) else {
                    let suggestions = suggest(name, param_names.iter().map(String::as_str));
                    return Err(match suggestions.first() {
                        Some(hint) => format!(
                            "Unknown named argument `{}` for {} — did you mean `{}`?",
                            name, callee_label, hint
                        ),
                        None => format!("Unknown named argument `{}` for {}", name, callee_label),
                    });
                };
                if slots[idx].is_some() {
                    return Err(format!(
                        "Argument for parameter `{}` of {} provided more than once",
                        param_names[idx], callee_label
                    ));
                }
                slots[idx] = Some((**value).clone());
            }
            other => {
                if next_pos >= param_names.len() {
                    return Err(format!(
                        "Too many positional arguments for {} (expected {}, got more)",
                        callee_label,
                        param_names.len()
                    ));
                }
                if slots[next_pos].is_some() {
                    return Err(format!(
                        "Argument for parameter `{}` of {} provided more than once",
                        param_names[next_pos], callee_label
                    ));
                }
                slots[next_pos] = Some(other.clone());
                next_pos += 1;
            }
        }
    }

    // Every slot must be filled, otherwise the caller missed a
    // parameter. The runtime previously left missing positional args
    // unbound silently; with named args we surface the error.
    let mut out = Vec::with_capacity(param_names.len());
    for (i, slot) in slots.into_iter().enumerate() {
        match slot {
            Some(node) => out.push(node),
            None => {
                return Err(format!(
                    "Missing argument for parameter `{}` of {}",
                    param_names[i], callee_label
                ));
            }
        }
    }
    Ok(out)
}

/// True if any element of `arguments` is a `Node::NamedArg`.
pub fn has_named(arguments: &[Node]) -> bool {
    arguments.iter().any(|n| matches!(n, Node::NamedArg { .. }))
}

/// Walk a parsed `Node::Program` and rewrite every CallExpression
/// whose callee is a known top-level fn so its `arguments` list is
/// in positional order. The pre-pass collects each top-level fn's
/// declared parameter names, then a single post-order rewrite
/// replaces named-arg lists with the resolved positional vector.
/// Returns the first resolution error (with `line:col`) if any
/// call's named args fail validation.
pub fn lower_program(program: &mut Node) -> Result<(), String> {
    let mut sigs: HashMap<String, Vec<String>> = HashMap::new();
    collect_signatures(program, &mut sigs);
    rewrite_calls(program, &sigs)
}

fn collect_signatures(node: &Node, sigs: &mut HashMap<String, Vec<String>>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                collect_signatures(&s.node, sigs);
            }
        }
        Node::Function {
            name, parameters, ..
        } => {
            // Top-level fn declarations register their parameter names
            // here so call-site lowering can find them. Methods inside
            // `impl` blocks come through the ImplBlock arm below.
            sigs.insert(
                name.clone(),
                parameters.iter().map(|(_t, n)| n.clone()).collect(),
            );
        }
        Node::ImplBlock { methods, .. } => {
            // Methods get registered under their mangled `Struct$method`
            // name, matching the eval-time lookup. The `self` parameter
            // is stripped so the user-visible parameter set lines up
            // with the call-site label set.
            for m in methods {
                if let Node::Function {
                    name, parameters, ..
                } = m
                {
                    let names: Vec<String> = parameters
                        .iter()
                        .skip(
                            if parameters
                                .first()
                                .map(|(_, n)| n == "self")
                                .unwrap_or(false)
                            {
                                1
                            } else {
                                0
                            },
                        )
                        .map(|(_t, n)| n.clone())
                        .collect();
                    sigs.insert(name.clone(), names);
                }
            }
        }
        _ => {}
    }
}

/// Post-order rewrite: visit children first so nested calls have
/// their named args lowered before the outer call inspects them.
/// Errors short-circuit at the first failure with a `line:col`
/// prefix when the named arg carries a span.
fn rewrite_calls(node: &mut Node, sigs: &HashMap<String, Vec<String>>) -> Result<(), String> {
    match node {
        Node::Program(stmts) => {
            for s in stmts.iter_mut() {
                rewrite_calls(&mut s.node, sigs)?;
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts.iter_mut() {
                rewrite_calls(s, sigs)?;
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            rewrite_calls(body, sigs)?;
            for r in requires.iter_mut() {
                rewrite_calls(r, sigs)?;
            }
            for e in ensures.iter_mut() {
                rewrite_calls(e, sigs)?;
            }
            if let Some(rec) = recovers_to {
                rewrite_calls(rec, sigs)?;
            }
        }
        Node::FunctionLiteral {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            rewrite_calls(body, sigs)?;
            for r in requires.iter_mut() {
                rewrite_calls(r, sigs)?;
            }
            for e in ensures.iter_mut() {
                rewrite_calls(e, sigs)?;
            }
            if let Some(rec) = recovers_to {
                rewrite_calls(rec, sigs)?;
            }
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods.iter_mut() {
                rewrite_calls(m, sigs)?;
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. }
        | Node::ExpressionStatement { expr: value, .. } => rewrite_calls(value, sigs)?,
        Node::ReturnStatement { value: Some(v), .. } => {
            rewrite_calls(v, sigs)?;
        }
        Node::ReturnStatement { value: None, .. } => {}
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            rewrite_calls(condition, sigs)?;
            rewrite_calls(consequence, sigs)?;
            if let Some(alt) = alternative {
                rewrite_calls(alt, sigs)?;
            }
        }
        Node::WhileStatement {
            condition,
            body,
            invariants,
            ..
        } => {
            rewrite_calls(condition, sigs)?;
            rewrite_calls(body, sigs)?;
            for inv in invariants.iter_mut() {
                rewrite_calls(inv, sigs)?;
            }
        }
        Node::ForInStatement {
            iterable,
            body,
            invariants,
            ..
        } => {
            rewrite_calls(iterable, sigs)?;
            rewrite_calls(body, sigs)?;
            for inv in invariants.iter_mut() {
                rewrite_calls(inv, sigs)?;
            }
        }
        Node::PrefixExpression { right, .. } => rewrite_calls(right, sigs)?,
        Node::InfixExpression { left, right, .. } => {
            rewrite_calls(left, sigs)?;
            rewrite_calls(right, sigs)?;
        }
        Node::IndexExpression { target, index, .. } => {
            rewrite_calls(target, sigs)?;
            rewrite_calls(index, sigs)?;
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            rewrite_calls(target, sigs)?;
            rewrite_calls(index, sigs)?;
            rewrite_calls(value, sigs)?;
        }
        Node::FieldAccess { target, .. } => rewrite_calls(target, sigs)?,
        Node::FieldAssignment { target, value, .. } => {
            rewrite_calls(target, sigs)?;
            rewrite_calls(value, sigs)?;
        }
        Node::ArrayLiteral { items, .. } => {
            for it in items.iter_mut() {
                rewrite_calls(it, sigs)?;
            }
        }
        Node::MapLiteral { entries, .. } => {
            for (k, v) in entries.iter_mut() {
                rewrite_calls(k, sigs)?;
                rewrite_calls(v, sigs)?;
            }
        }
        Node::SetLiteral { items, .. } => {
            for it in items.iter_mut() {
                rewrite_calls(it, sigs)?;
            }
        }
        Node::TryExpression { expr, .. } => rewrite_calls(expr, sigs)?,
        Node::OptionalChain { object, access, .. } => {
            rewrite_calls(object, sigs)?;
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args.iter_mut() {
                    rewrite_calls(a, sigs)?;
                }
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            rewrite_calls(scrutinee, sigs)?;
            for (_pat, guard, body) in arms.iter_mut() {
                if let Some(g) = guard {
                    rewrite_calls(g, sigs)?;
                }
                rewrite_calls(body, sigs)?;
            }
        }
        Node::TryCatch { body, handlers, .. } => {
            for s in body.iter_mut() {
                rewrite_calls(s, sigs)?;
            }
            for (_v, handler_body) in handlers.iter_mut() {
                for s in handler_body.iter_mut() {
                    rewrite_calls(s, sigs)?;
                }
            }
        }
        Node::Quantifier { range, body, .. } => {
            match range {
                crate::quantifiers::QuantRange::Range { lo, hi } => {
                    rewrite_calls(lo, sigs)?;
                    rewrite_calls(hi, sigs)?;
                }
                crate::quantifiers::QuantRange::Iterable(expr) => rewrite_calls(expr, sigs)?,
            }
            rewrite_calls(body, sigs)?;
        }
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            rewrite_calls(condition, sigs)?;
            if let Some(m) = message {
                rewrite_calls(m, sigs)?;
            }
        }
        Node::InvariantStatement { expr, .. } => rewrite_calls(expr, sigs)?,
        Node::LiveBlock {
            body, invariants, ..
        } => {
            rewrite_calls(body, sigs)?;
            for inv in invariants.iter_mut() {
                rewrite_calls(inv, sigs)?;
            }
        }
        Node::StructLiteral { fields, .. } => {
            for (_n, v) in fields.iter_mut() {
                rewrite_calls(v, sigs)?;
            }
        }
        // The CallExpression arm: lower nested calls first, then
        // rewrite this call's argument list if it carries any
        // named args and we know the callee's parameter names.
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            rewrite_calls(function, sigs)?;
            for a in arguments.iter_mut() {
                rewrite_calls(a, sigs)?;
            }
            if !has_named(arguments) {
                return Ok(());
            }
            // Figure out the callee name we should look up. Two
            // shapes carry resolvable names: a bare identifier
            // (`f(...)`) and a struct method call (`obj.m(...)`)
            // which goes through FieldAccess and the mangled
            // `Struct$m` name. The latter is rewritten by the
            // interpreter at dispatch — at parse time we only have
            // the method name, not the struct type, so we can't
            // resolve it eagerly here. Calls on other shapes keep
            // their NamedArg nodes; the runtime path errors out.
            let callee_name = if let Node::Identifier { name, .. } = function.as_ref() {
                Some(name.clone())
            } else {
                None
            };
            if let Some(name) = callee_name
                && let Some(param_names) = sigs.get(&name)
            {
                let label = format!("fn `{}`", name);
                let lowered = resolve(&label, param_names, arguments)?;
                *arguments = lowered;
            }
            // Otherwise leave NamedArg nodes in place; the runtime
            // path raises a clean diagnostic if the call ever
            // reaches eval with named args still present.
        }
        // Leaves and statements that don't carry sub-expressions:
        // nothing to recurse into.
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    fn lit(v: i64) -> Node {
        Node::IntegerLiteral {
            value: v,
            span: Span::default(),
        }
    }
    fn named(name: &str, v: i64) -> Node {
        Node::NamedArg {
            name: name.to_string(),
            value: Box::new(lit(v)),
            span: Span::default(),
        }
    }

    fn params(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn all_named_in_reverse_order_reorders_to_param_order() {
        let p = params(&["x", "y", "z"]);
        let args = vec![named("z", 30), named("y", 20), named("x", 10)];
        let out = resolve("fn f", &p, &args).unwrap();
        match &out[0] {
            Node::IntegerLiteral { value, .. } => assert_eq!(*value, 10),
            _ => panic!("slot 0 should be x=10"),
        }
        match &out[1] {
            Node::IntegerLiteral { value, .. } => assert_eq!(*value, 20),
            _ => panic!("slot 1 should be y=20"),
        }
        match &out[2] {
            Node::IntegerLiteral { value, .. } => assert_eq!(*value, 30),
            _ => panic!("slot 2 should be z=30"),
        }
    }

    #[test]
    fn positional_then_named_fills_correctly() {
        let p = params(&["x", "y", "z"]);
        let args = vec![lit(1), named("z", 3), named("y", 2)];
        let out = resolve("fn f", &p, &args).unwrap();
        match &out[2] {
            Node::IntegerLiteral { value, .. } => assert_eq!(*value, 3),
            _ => panic!("slot 2 should be z=3"),
        }
    }

    #[test]
    fn unknown_named_reports_with_suggestion() {
        let p = params(&["timeout", "retries"]);
        let args = vec![named("timout", 5)];
        let err = resolve("fn f", &p, &args).unwrap_err();
        assert!(err.contains("timout"), "diagnostic: {}", err);
        assert!(err.contains("did you mean"), "diagnostic: {}", err);
    }

    #[test]
    fn duplicate_target_via_positional_and_named_errors() {
        let p = params(&["x", "y"]);
        let args = vec![lit(1), named("x", 99)];
        let err = resolve("fn f", &p, &args).unwrap_err();
        assert!(
            err.contains("provided more than once"),
            "diagnostic: {}",
            err
        );
    }

    #[test]
    fn missing_param_after_named_resolution_errors() {
        let p = params(&["x", "y"]);
        let args = vec![named("y", 2)];
        let err = resolve("fn f", &p, &args).unwrap_err();
        assert!(err.contains("Missing argument"), "diagnostic: {}", err);
    }
}
