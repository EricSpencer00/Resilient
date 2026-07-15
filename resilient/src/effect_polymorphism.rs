//! RES-3933 A-E7: effect enforcement for higher-order functions.
//!
//! `check_program_effects` (RES-389, in `typechecker.rs`) already
//! enforces that a `pure` fn only calls other `pure` fns/builtins.
//! It has no idea what to do with a call through a *function-typed
//! parameter* though — `fn run(fn(int) -> int f, int x) -> int { f(x) }`
//! — so it falls back to a blanket "unknown callee" rejection. That
//! rejection is sound (never lets an unproven-safe call through) but
//! it is also so coarse it rejects *every* higher-order call from a
//! `pure` fn, including ones that are provably safe (e.g. a `pure`
//! HOF called with a `pure` named-function callback). That defeats
//! the point of writing HOFs at all.
//!
//! This module narrows the rejection to genuine violations:
//!
//! 1. For each `pure`-declared fn, find its function-typed
//!    parameters that are actually **invoked** in the body
//!    ([`invoked_callback_params`]).
//! 2. For each such parameter, scan every call site of the fn
//!    anywhere in the program and inspect the argument bound to that
//!    parameter position.
//! 3. If the argument is a plain identifier naming a **top-level
//!    fn whose declared effect is not `pure`** (explicit `io fn`, or
//!    unannotated — which defaults to `io` per RES-389), that's a
//!    proven violation: the HOF would invoke an io-effect callback
//!    from a pure context. Reject with a `line:col` diagnostic
//!    pointing at the call site.
//! 4. Anything else the argument could be — an inline lambda
//!    literal, a local variable, a field/index expression, the
//!    result of another call — has no statically-known effect here.
//!    Per the project's "when uncertain, accept" rule for this
//!    increment, those call sites are **not** flagged. This is the
//!    monomorphic, sound-but-incomplete direction: we only ever
//!    reject a *proven* violation, never an unproven one.
//!
//! `check_body_effects` in `typechecker.rs` cooperates: instead of
//! hard-rejecting every call through a function-typed parameter of
//! the enclosing fn, it now defers to this module (see the
//! `is_function_type` check inlined there) and lets [`check`] make
//! the final call using whole-program call-site information the
//! per-body walk doesn't have.
//!
//! ## Deferred (tracked as a follow-up issue, see the PR body)
//!
//! True effect-variable polymorphism — `fn run<E>(f: () -> int ! E)
//! -> int ! E` unifying `E` with the actual callback's effect so
//! `run` itself is `pure` when given a `pure` callback and `io` when
//! given an `io` one — is *not* implemented. RES-193 already parses
//! the `-e->` effect-arrow annotation on function-typed parameters
//! but nothing consumes it yet; that's the real generalization this
//! module intentionally leaves on the table. What's here is the
//! smallest sound slice: monomorphic HOFs (no effect-variable
//! generics) get real enforcement instead of "reject everything".
//! - Inline lambda-literal callback arguments are never inspected
//!   for an explicit `pure`/`io` annotation, even though the parser
//!   supports one — only plain-identifier arguments naming a
//!   top-level fn are resolved. Extending this to lambda literals
//!   requires reliably distinguishing an *explicit* `pure`/`io`
//!   annotation on the literal from the default (`io`) so an
//!   unannotated inline closure isn't wrongly flagged.
//! - Local variables bound to a function value (`let g = ...; run(g, x)`)
//!   are not traced back to their initializer.
//! - Callback parameters invoked indirectly (through a method call,
//!   stored in a struct field, etc.) are out of scope.

use crate::span::Spanned;
use crate::{EffectSet, Node};
use std::collections::{HashMap, HashSet};

/// True when a (possibly `linear `-prefixed) type-annotation string
/// denotes a function type, i.e. it was parsed via the RES-403
/// `fn(T1, T2, ...) -> R` / RES-193 `fn(...) -e-> R` grammar and
/// therefore starts with `fn(` once any `linear` prefix is stripped.
pub(crate) fn is_function_type(ty: &str) -> bool {
    let t = ty.trim();
    let t = t.strip_prefix("linear ").map(str::trim_start).unwrap_or(t);
    t.starts_with("fn(")
}

/// Top-level entry, called from `typechecker::check_program_effects`
/// after it has built `fn_effects` (name -> declared `EffectSet` for
/// every top-level fn). Returns the first proven violation found.
pub(crate) fn check(
    statements: &[Spanned<Node>],
    fn_effects: &HashMap<String, EffectSet>,
    source_path: &str,
) -> Result<(), String> {
    for stmt in statements {
        let Node::Function {
            name,
            parameters,
            body,
            effects,
            ..
        } = &stmt.node
        else {
            continue;
        };
        if !effects.pure {
            continue;
        }
        let invoked = invoked_callback_params(parameters, body);
        if invoked.is_empty() {
            continue;
        }
        for (idx, (ty, pname)) in parameters.iter().enumerate() {
            if !is_function_type(ty) || !invoked.contains(pname.as_str()) {
                continue;
            }
            check_call_sites(statements, name, idx, pname, fn_effects, source_path)?;
        }
    }
    Ok(())
}

/// Names of `parameters` that are (a) function-typed and (b) called
/// somewhere inside `body` — i.e. the callback parameters whose
/// bound value's effect actually matters for this fn's purity.
/// A function-typed parameter the body never invokes (just passes
/// along, stores, etc.) doesn't constrain the caller at all.
fn invoked_callback_params(parameters: &[(String, String)], body: &Node) -> HashSet<String> {
    let fn_typed: HashSet<&str> = parameters
        .iter()
        .filter(|(ty, _)| is_function_type(ty))
        .map(|(_, n)| n.as_str())
        .collect();
    if fn_typed.is_empty() {
        return HashSet::new();
    }
    let mut invoked = HashSet::new();
    collect_invoked(body, &fn_typed, &mut invoked);
    invoked.into_iter().map(String::from).collect()
}

/// Recursive walk collecting every name in `candidates` that appears
/// as the callee of a `CallExpression` anywhere under `node`. Mirrors
/// the node coverage of `typechecker::check_body_effects` so nested
/// blocks/branches/loops are all visited.
fn collect_invoked<'a>(node: &'a Node, candidates: &HashSet<&str>, out: &mut HashSet<&'a str>) {
    match node {
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_invoked(s, candidates, out);
            }
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            collect_invoked(value, candidates, out);
        }
        Node::ReturnStatement { value: Some(v), .. } => collect_invoked(v, candidates, out),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_invoked(condition, candidates, out);
            collect_invoked(consequence, candidates, out);
            if let Some(a) = alternative {
                collect_invoked(a, candidates, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_invoked(condition, candidates, out);
            collect_invoked(body, candidates, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_invoked(iterable, candidates, out);
            collect_invoked(body, candidates, out);
        }
        Node::Assert { condition, .. } | Node::Assume { condition, .. } => {
            collect_invoked(condition, candidates, out);
        }
        Node::LiveBlock { body, .. } => collect_invoked(body, candidates, out),
        Node::InfixExpression { left, right, .. } => {
            collect_invoked(left, candidates, out);
            collect_invoked(right, candidates, out);
        }
        Node::PrefixExpression { right, .. } => collect_invoked(right, candidates, out),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref()
                && candidates.contains(name.as_str())
            {
                out.insert(name.as_str());
            }
            collect_invoked(function, candidates, out);
            for a in arguments {
                collect_invoked(a, candidates, out);
            }
        }
        Node::FieldAccess { target, .. } => collect_invoked(target, candidates, out),
        Node::FieldAssignment { target, value, .. } => {
            collect_invoked(target, candidates, out);
            collect_invoked(value, candidates, out);
        }
        Node::Assignment { value, .. } => collect_invoked(value, candidates, out),
        Node::IndexExpression { target, index, .. } => {
            collect_invoked(target, candidates, out);
            collect_invoked(index, candidates, out);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            collect_invoked(target, candidates, out);
            collect_invoked(index, candidates, out);
            collect_invoked(value, candidates, out);
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                collect_invoked(i, candidates, out);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(b) = base {
                collect_invoked(b, candidates, out);
            }
            for (_, v) in fields {
                collect_invoked(v, candidates, out);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_invoked(scrutinee, candidates, out);
            for (_pat, guard, arm_body) in arms {
                if let Some(g) = guard {
                    collect_invoked(g, candidates, out);
                }
                collect_invoked(arm_body, candidates, out);
            }
        }
        Node::ExpressionStatement { expr, .. } => collect_invoked(expr, candidates, out),
        Node::TryExpression { expr, .. } => collect_invoked(expr, candidates, out),
        Node::OptionalChain { object, access, .. } => {
            collect_invoked(object, candidates, out);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    collect_invoked(a, candidates, out);
                }
            }
        }
        Node::Function { body, .. } => collect_invoked(body, candidates, out),
        _ => {}
    }
}

/// Scan every statement in the program for a call to `hof_name` and
/// check the argument bound to parameter index `idx`. Returns the
/// first proven violation (a plain identifier naming a non-`pure`
/// top-level fn) as a fully-formatted diagnostic string.
fn check_call_sites(
    statements: &[Spanned<Node>],
    hof_name: &str,
    idx: usize,
    param_name: &str,
    fn_effects: &HashMap<String, EffectSet>,
    source_path: &str,
) -> Result<(), String> {
    let mut violation: Option<(crate::span::Span, String)> = None;
    for stmt in statements {
        find_hof_call_violations(&stmt.node, hof_name, idx, fn_effects, &mut violation);
        if violation.is_some() {
            break;
        }
    }
    match violation {
        Some((span, callback_name)) => {
            let (line, col) = (span.start.line, span.start.column);
            Err(if line == 0 {
                format!(
                    "cannot pass io callback `{callback_name}` to pure higher-order function \
                     `{hof_name}` (parameter `{param_name}` is invoked inside `{hof_name}`)"
                )
            } else {
                format!(
                    "{source_path}:{line}:{col}: cannot pass io callback `{callback_name}` to \
                     pure higher-order function `{hof_name}` (parameter `{param_name}` is \
                     invoked inside `{hof_name}`)"
                )
            })
        }
        None => Ok(()),
    }
}

/// Recursive walk mirroring [`collect_invoked`], but instead of
/// collecting invoked parameter names it looks for `CallExpression`
/// nodes that call `hof_name` and checks the argument at `idx`.
fn find_hof_call_violations(
    node: &Node,
    hof_name: &str,
    idx: usize,
    fn_effects: &HashMap<String, EffectSet>,
    violation: &mut Option<(crate::span::Span, String)>,
) {
    if violation.is_some() {
        return;
    }
    match node {
        Node::Block { stmts, .. } => {
            for s in stmts {
                find_hof_call_violations(s, hof_name, idx, fn_effects, violation);
            }
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            find_hof_call_violations(value, hof_name, idx, fn_effects, violation);
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            find_hof_call_violations(v, hof_name, idx, fn_effects, violation);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            find_hof_call_violations(condition, hof_name, idx, fn_effects, violation);
            find_hof_call_violations(consequence, hof_name, idx, fn_effects, violation);
            if let Some(a) = alternative {
                find_hof_call_violations(a, hof_name, idx, fn_effects, violation);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            find_hof_call_violations(condition, hof_name, idx, fn_effects, violation);
            find_hof_call_violations(body, hof_name, idx, fn_effects, violation);
        }
        Node::ForInStatement { iterable, body, .. } => {
            find_hof_call_violations(iterable, hof_name, idx, fn_effects, violation);
            find_hof_call_violations(body, hof_name, idx, fn_effects, violation);
        }
        Node::Assert { condition, .. } | Node::Assume { condition, .. } => {
            find_hof_call_violations(condition, hof_name, idx, fn_effects, violation);
        }
        Node::LiveBlock { body, .. } => {
            find_hof_call_violations(body, hof_name, idx, fn_effects, violation);
        }
        Node::InfixExpression { left, right, .. } => {
            find_hof_call_violations(left, hof_name, idx, fn_effects, violation);
            find_hof_call_violations(right, hof_name, idx, fn_effects, violation);
        }
        Node::PrefixExpression { right, .. } => {
            find_hof_call_violations(right, hof_name, idx, fn_effects, violation);
        }
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            if let Node::Identifier { name, .. } = function.as_ref()
                && name == hof_name
                && let Some(arg) = arguments.get(idx)
                && let Node::Identifier {
                    name: callback_name,
                    ..
                } = arg
                && let Some(effects) = fn_effects.get(callback_name)
                && !effects.pure
            {
                *violation = Some((*span, callback_name.clone()));
                return;
            }
            find_hof_call_violations(function, hof_name, idx, fn_effects, violation);
            for a in arguments {
                find_hof_call_violations(a, hof_name, idx, fn_effects, violation);
            }
        }
        Node::FieldAccess { target, .. } => {
            find_hof_call_violations(target, hof_name, idx, fn_effects, violation);
        }
        Node::FieldAssignment { target, value, .. } => {
            find_hof_call_violations(target, hof_name, idx, fn_effects, violation);
            find_hof_call_violations(value, hof_name, idx, fn_effects, violation);
        }
        Node::Assignment { value, .. } => {
            find_hof_call_violations(value, hof_name, idx, fn_effects, violation);
        }
        Node::IndexExpression { target, index, .. } => {
            find_hof_call_violations(target, hof_name, idx, fn_effects, violation);
            find_hof_call_violations(index, hof_name, idx, fn_effects, violation);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            find_hof_call_violations(target, hof_name, idx, fn_effects, violation);
            find_hof_call_violations(index, hof_name, idx, fn_effects, violation);
            find_hof_call_violations(value, hof_name, idx, fn_effects, violation);
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                find_hof_call_violations(i, hof_name, idx, fn_effects, violation);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(b) = base {
                find_hof_call_violations(b, hof_name, idx, fn_effects, violation);
            }
            for (_, v) in fields {
                find_hof_call_violations(v, hof_name, idx, fn_effects, violation);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            find_hof_call_violations(scrutinee, hof_name, idx, fn_effects, violation);
            for (_pat, guard, arm_body) in arms {
                if let Some(g) = guard {
                    find_hof_call_violations(g, hof_name, idx, fn_effects, violation);
                }
                find_hof_call_violations(arm_body, hof_name, idx, fn_effects, violation);
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            find_hof_call_violations(expr, hof_name, idx, fn_effects, violation);
        }
        Node::TryExpression { expr, .. } => {
            find_hof_call_violations(expr, hof_name, idx, fn_effects, violation);
        }
        Node::OptionalChain { object, access, .. } => {
            find_hof_call_violations(object, hof_name, idx, fn_effects, violation);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    find_hof_call_violations(a, hof_name, idx, fn_effects, violation);
                }
            }
        }
        Node::Function { body, .. } => {
            find_hof_call_violations(body, hof_name, idx, fn_effects, violation);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn stmts(src: &str) -> Vec<Spanned<Node>> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        match prog {
            Node::Program(s) => s,
            other => panic!("expected Program, got {:?}", other),
        }
    }

    fn effects_of(statements: &[Spanned<Node>]) -> HashMap<String, EffectSet> {
        let mut out = HashMap::new();
        for s in statements {
            if let Node::Function { name, effects, .. } = &s.node {
                out.insert(name.clone(), *effects);
            }
        }
        out
    }

    #[test]
    fn is_function_type_recognizes_fn_types() {
        assert!(is_function_type("fn(int) -> int"));
        assert!(is_function_type("linear fn(int) -> int"));
        assert!(!is_function_type("int"));
        assert!(!is_function_type("Array<int>"));
    }

    #[test]
    fn pure_hof_rejects_io_callback() {
        let src = "io fn noisy(int x) -> int { println(x); return x; }\n\
                   pure fn run(fn(int) -> int f, int x) -> int { return f(x); }\n\
                   fn caller() -> int { return run(noisy, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let err = check(&s, &fe, "<t>").expect_err("io callback into pure HOF must be rejected");
        assert!(
            err.contains("cannot pass io callback `noisy` to pure higher-order function `run`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pure_hof_accepts_pure_callback() {
        let src = "pure fn add1(int x) -> int { return x + 1; }\n\
                   pure fn run(fn(int) -> int f, int x) -> int { return f(x); }\n\
                   fn caller() -> int { return run(add1, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        check(&s, &fe, "<t>").expect("pure callback into pure HOF must be accepted");
    }

    #[test]
    fn pure_hof_with_unresolvable_callback_stays_permissive() {
        // The callback argument is a local variable, not a
        // plain top-level fn name, so its effect can't be proven.
        // Per "when uncertain, accept" this must not be rejected.
        let src = "pure fn run(fn(int) -> int f, int x) -> int { return f(x); }\n\
                   fn caller(fn(int) -> int cb) -> int { return run(cb, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        check(&s, &fe, "<t>").expect("unresolvable callback must stay permissive");
    }

    #[test]
    fn io_hof_with_io_callback_is_never_checked() {
        // Only `pure`-declared HOFs are constrained; an `io` HOF is
        // free to invoke any callback effect.
        let src = "io fn noisy(int x) -> int { println(x); return x; }\n\
                   io fn run(fn(int) -> int f, int x) -> int { return f(x); }\n\
                   fn caller() -> int { return run(noisy, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        check(&s, &fe, "<t>").expect("io HOF is not constrained by this pass");
    }

    #[test]
    fn unannotated_hof_is_never_checked() {
        let src = "io fn noisy(int x) -> int { println(x); return x; }\n\
                   fn run(fn(int) -> int f, int x) -> int { return f(x); }\n\
                   fn caller() -> int { return run(noisy, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        check(&s, &fe, "<t>").expect("unannotated HOF is not constrained by this pass");
    }

    #[test]
    fn pure_hof_with_non_invoked_fn_param_is_never_checked() {
        // `f` is passed along but never called inside `run`'s body —
        // nothing to enforce.
        let src = "io fn noisy(int x) -> int { println(x); return x; }\n\
                   pure fn identity(fn(int) -> int f) -> fn(int) -> int { return f; }\n\
                   fn caller() -> fn(int) -> int { return identity(noisy); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        check(&s, &fe, "<t>").expect("non-invoked fn-typed param is never checked");
    }

    #[test]
    fn diagnostic_includes_source_location() {
        let src = "io fn noisy(int x) -> int { println(x); return x; }\n\
                   pure fn run(fn(int) -> int f, int x) -> int { return f(x); }\n\
                   fn caller() -> int {\n    return run(noisy, 5);\n}\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let err = check(&s, &fe, "<t>").unwrap_err();
        assert!(err.contains("<t>:"), "missing source path: {err}");
        assert!(err.contains(":4:"), "missing line number: {err}");
    }
}
