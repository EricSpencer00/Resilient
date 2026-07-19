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
//! ## True effect-variable polymorphism (RES-3933 A-E7 follow-up, #4072)
//!
//! [`resolves_pure_at_call_site`] extends the above to a genuinely
//! effect-*polymorphic* HOF — one that never declares `pure fn`/`io
//! fn` at all because its effect legitimately depends on what's
//! passed to it:
//!
//! ```text
//! fn run(f: fn(int) -e-> int, x: int) -> int {
//!     return f(x);
//! }
//! ```
//!
//! RES-193 already parses the `-e->` effect-arrow on a function-typed
//! parameter (marking it an "effect-variable parameter"); this module
//! now consumes that marker: when a `pure` caller calls `run`, and
//! `run`'s own declared effect defaults to `io` (unannotated), the
//! call is proven pure anyway when (a) the argument bound to `f` at
//! *this* call site is a plain identifier naming a provably-`pure`
//! top-level fn, and (b) every other operation in `run`'s body —
//! everything apart from invoking `f` — is independently pure
//! ([`body_pure_modulo`]). `run(add1, 5)` type-checks as pure when
//! `add1` is `pure`; `run(noisy, 5)` is still rejected when `noisy`
//! is `io` (the existing unconditional-rejection path is untouched).
//! Like [`check`], this can only ever *accept* a call that was
//! previously rejected — never reject one that used to pass.
//!
//! ## Local-variable callback tracing (RES-3933 A-E7 follow-up, #4097)
//!
//! [`resolve_local_alias`] extends both [`check`] and
//! [`resolves_pure_at_call_site`] to trace a callback argument that is
//! a *local variable* back to its initializer — `let g = add1; run(g,
//! 5);` now resolves `g` to `add1` in both the monomorphic
//! (`check`/`find_hof_call_violations`) and effect-polymorphic
//! (`resolves_pure_at_call_site`) directions, using the enclosing
//! top-level fn's body (threaded through as `caller_body`) as the
//! scope to search. The trace is deliberately conservative: it only
//! fires when there is exactly one `let g = <identifier>;` binding for
//! that name anywhere in the enclosing fn, *and* `g` is never the
//! target of a plain `Assignment` anywhere in that fn (path-insensitive
//! — any reassignment anywhere disqualifies the trace, even one that
//! happens after the call site). Either condition failing leaves the
//! argument unresolvable, same as before this pass existed — "when
//! uncertain, don't rescue/reject" is preserved exactly.
//!
//! ## Inline lambda-literal callback resolution (RES-3933 A-E7, #4123)
//!
//! The grammar now parses `pure fn(...) { ... }` / `io fn(...) { ... }`
//! as an *expression* — `parse_function_literal_with_effect` in
//! `lib.rs` records the explicit annotation on `Node::FunctionLiteral`'s
//! new `explicit_effect: Option<EffectSet>` field. `explicit_effect`
//! is `None` for a bare, unannotated `fn(...) { ... }` literal — that
//! case is deliberately left unresolved everywhere below, exactly as
//! before this pass — and `Some(EffectSet)` only when the source wrote
//! `pure`/`io` immediately before `fn`.
//!
//! Both [`find_hof_call_violations`] (the monomorphic direction) and
//! [`resolves_pure_at_call_site`] (the effect-polymorphic direction)
//! now inspect a call argument that is directly a `FunctionLiteral`:
//! an explicit `io fn(...)` argument to a `pure`-declared HOF is a
//! proven violation (same diagnostic path as a non-pure named-fn
//! argument); an explicit `pure fn(...)` argument to an effect-
//! polymorphic HOF is treated exactly like a pure named-fn argument
//! for the unification check. An inline literal is still never
//! resolved through a local-variable alias or nested one level deeper
//! (e.g. stored in a struct field then invoked) — those remain out of
//! scope below.
//!
//! ## Named effect-variable unification (RES-4123)
//!
//! `EffectSet::effect_var` (RES-4123, `lib.rs`) now preserves a fn's
//! own `-e-> TYPE` return-type letter past the parser instead of
//! discarding it. Separately, [`effect_arrow_letter`] extracts the
//! per-parameter `-e->` letter that was already embedded in a
//! function-typed parameter's type-annotation *string* (e.g.
//! `"fn(int) -e-> int"`) since RES-193 — no parser change was needed
//! for that half, since the letter already survives there.
//!
//! [`check_effect_var_unification`] uses the per-parameter letters to
//! catch a genuinely new class of *proven* violation: two or more of a
//! HOF's invoked, function-typed parameters that share the same
//! effect-variable letter are required by the named-effect-variable
//! contract to carry the *same* effect at every call site (that's what
//! sharing a letter means — `fn combine(fn(int) -e-> int f, fn(int)
//! -e-> int g, ...)` says "f and g have the same, as-yet-unknown,
//! effect", not "f and g may each independently be pure or io"). A
//! call site that binds one same-letter parameter to a provably `pure`
//! argument and another to a provably `io` argument can never be
//! satisfied by any single effect and is rejected with a `line:col`
//! diagnostic. Two parameters with *different* letters (`-e->` vs
//! `-f->`) are never compared against each other — they're
//! independent effect variables by construction, exactly like two
//! independent generic type parameters `T` and `U`.
//!
//! As with every other check in this module, this can only ever
//! reject a *provable* violation: both arguments must independently
//! resolve to a concrete effect (via the same identifier / local-alias
//! / explicit-lambda-literal resolution used elsewhere in this file)
//! before they're compared. Any call site where one or both same-letter
//! arguments are unresolvable is left unchecked — "when uncertain,
//! accept" applies here exactly as it does to the rescue-direction
//! checks above. This check runs for *every* top-level fn (not just
//! `pure`-declared ones), because the requirement comes from the
//! callee's own named-effect-variable signature, independent of
//! whatever the calling fn's declared effect happens to be.
//!
//! ## Deferred (tracked as a follow-up issue, see the PR body)
//!
//! - Callback parameters invoked indirectly (through a method call,
//!   stored in a struct field, etc.) are out of scope.
//! - Unification only compares parameters *within a single call site*;
//!   it does not attempt to unify a `let`-bound effect-polymorphic
//!   partial application, or propagate a resolved letter's effect back
//!   through nested HOF calls (effect-variable letters aren't threaded
//!   transitively across HOF boundaries).
//! - Full row-polymorphism / whole-program effect inference remains
//!   out of scope, per the parent ticket.

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
    // RES-4123: named effect-variable unification. Runs for every
    // top-level fn regardless of its own declared effect — see the
    // module doc, "Named effect-variable unification".
    check_effect_var_unification(statements, fn_effects, source_path)?;
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
        find_hof_call_violations(
            &stmt.node,
            hof_name,
            idx,
            fn_effects,
            &stmt.node,
            &mut violation,
        );
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
    caller_body: &Node,
    violation: &mut Option<(crate::span::Span, String)>,
) {
    if violation.is_some() {
        return;
    }
    match node {
        Node::Block { stmts, .. } => {
            for s in stmts {
                find_hof_call_violations(s, hof_name, idx, fn_effects, caller_body, violation);
            }
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            find_hof_call_violations(value, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            find_hof_call_violations(v, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            find_hof_call_violations(condition, hof_name, idx, fn_effects, caller_body, violation);
            find_hof_call_violations(
                consequence,
                hof_name,
                idx,
                fn_effects,
                caller_body,
                violation,
            );
            if let Some(a) = alternative {
                find_hof_call_violations(a, hof_name, idx, fn_effects, caller_body, violation);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            find_hof_call_violations(condition, hof_name, idx, fn_effects, caller_body, violation);
            find_hof_call_violations(body, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::ForInStatement { iterable, body, .. } => {
            find_hof_call_violations(iterable, hof_name, idx, fn_effects, caller_body, violation);
            find_hof_call_violations(body, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::Assert { condition, .. } | Node::Assume { condition, .. } => {
            find_hof_call_violations(condition, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::LiveBlock { body, .. } => {
            find_hof_call_violations(body, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::InfixExpression { left, right, .. } => {
            find_hof_call_violations(left, hof_name, idx, fn_effects, caller_body, violation);
            find_hof_call_violations(right, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::PrefixExpression { right, .. } => {
            find_hof_call_violations(right, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            if let Node::Identifier { name, .. } = function.as_ref()
                && name == hof_name
                && let Some(arg) = arguments.get(idx)
            {
                match arg {
                    Node::Identifier {
                        name: callback_name,
                        ..
                    } => {
                        // RES-3933 A-E7 follow-up (#4097): a plain
                        // identifier argument that doesn't itself name
                        // a top-level fn might be a local variable
                        // aliasing one — trace it back through
                        // `resolve_local_alias` (sound only when the
                        // alias is unambiguous and never reassigned;
                        // see its doc comment) before falling back to
                        // treating it as unresolvable.
                        let resolved_name = if fn_effects.contains_key(callback_name) {
                            Some(callback_name.clone())
                        } else {
                            resolve_local_alias(callback_name, caller_body)
                        };
                        if let Some(resolved_name) = resolved_name
                            && let Some(effects) = fn_effects.get(&resolved_name)
                            && !effects.pure
                        {
                            *violation = Some((*span, resolved_name));
                            return;
                        }
                    }
                    // RES-3933 A-E7: an inline lambda literal argument
                    // with an *explicit* `io fn(...)` annotation is a
                    // proven violation — it's an io callback passed
                    // straight into a pure HOF's callback slot, no
                    // aliasing/resolution needed. An unannotated bare
                    // `fn(...)` literal (`explicit_effect: None`) is
                    // left unresolved, same as before this pass —
                    // only a *pure*-declared HOF's own body inspection
                    // could tell if the literal is safe, and that is
                    // out of scope here.
                    Node::FunctionLiteral {
                        explicit_effect: Some(effects),
                        ..
                    } if !effects.pure => {
                        *violation = Some((*span, "<inline io fn(...)>".to_string()));
                        return;
                    }
                    _ => {}
                }
            }
            find_hof_call_violations(function, hof_name, idx, fn_effects, caller_body, violation);
            for a in arguments {
                find_hof_call_violations(a, hof_name, idx, fn_effects, caller_body, violation);
            }
        }
        Node::FieldAccess { target, .. } => {
            find_hof_call_violations(target, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::FieldAssignment { target, value, .. } => {
            find_hof_call_violations(target, hof_name, idx, fn_effects, caller_body, violation);
            find_hof_call_violations(value, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::Assignment { value, .. } => {
            find_hof_call_violations(value, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::IndexExpression { target, index, .. } => {
            find_hof_call_violations(target, hof_name, idx, fn_effects, caller_body, violation);
            find_hof_call_violations(index, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            find_hof_call_violations(target, hof_name, idx, fn_effects, caller_body, violation);
            find_hof_call_violations(index, hof_name, idx, fn_effects, caller_body, violation);
            find_hof_call_violations(value, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                find_hof_call_violations(i, hof_name, idx, fn_effects, caller_body, violation);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(b) = base {
                find_hof_call_violations(b, hof_name, idx, fn_effects, caller_body, violation);
            }
            for (_, v) in fields {
                find_hof_call_violations(v, hof_name, idx, fn_effects, caller_body, violation);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            find_hof_call_violations(scrutinee, hof_name, idx, fn_effects, caller_body, violation);
            for (_pat, guard, arm_body) in arms {
                if let Some(g) = guard {
                    find_hof_call_violations(g, hof_name, idx, fn_effects, caller_body, violation);
                }
                find_hof_call_violations(
                    arm_body,
                    hof_name,
                    idx,
                    fn_effects,
                    caller_body,
                    violation,
                );
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            find_hof_call_violations(expr, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::TryExpression { expr, .. } => {
            find_hof_call_violations(expr, hof_name, idx, fn_effects, caller_body, violation);
        }
        Node::OptionalChain { object, access, .. } => {
            find_hof_call_violations(object, hof_name, idx, fn_effects, caller_body, violation);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    find_hof_call_violations(a, hof_name, idx, fn_effects, caller_body, violation);
                }
            }
        }
        Node::Function { body, .. } => {
            find_hof_call_violations(body, hof_name, idx, fn_effects, body, violation);
        }
        _ => {}
    }
}

/// RES-3933 A-E7 follow-up (#4072): true effect-variable
/// polymorphism.
///
/// [`check`] (above) only ever *narrows* the blanket rejection for
/// calls made *through* a `pure`-declared HOF's own function-typed
/// parameter. It says nothing about a HOF like
///
/// ```text
/// fn run(f: fn(int) -e-> int, x: int) -> int {
///     return f(x);
/// }
/// ```
///
/// which never declares `pure fn run` at all — its effect genuinely
/// *depends* on what's passed as `f`, so RES-193's `-e->` effect-arrow
/// marks `f` as an effect-variable parameter rather than fixing `run`
/// to a single effect. Before this function existed, every call to
/// `run` from a `pure` context was rejected outright (its declared
/// effect defaults to `io` per RES-389), even `run(add1, 5)` where
/// `add1` is provably `pure` — defeating the entire point of writing
/// a genuinely effect-polymorphic HOF.
///
/// This function proves a *specific call site* `callee_name(call_arguments)`
/// is pure despite `callee_name`'s own declared effect not being
/// `pure`, by checking all of:
///
/// 1. `callee_name` has one or more invoked, function-typed
///    parameters whose type annotation carries the RES-193 `-e->`
///    effect-arrow marker (an "effect-variable parameter").
/// 2. Every argument bound to one of those parameters *at this call
///    site* is a plain identifier naming a top-level fn that is
///    provably `pure`.
/// 3. Every other operation in `callee_name`'s body — everything
///    apart from invoking one of those effect-variable parameters —
///    independently satisfies the same rules `check_body_effects`
///    enforces for an explicitly `pure fn` ([`body_pure_modulo`]).
///
/// Returns `false` (never rescues) whenever any of the above can't be
/// proven: no qualifying parameter, an unresolvable/non-pure argument,
/// or a body operation that isn't provably pure. The caller
/// (`typechecker::check_body_effects`) only calls this after its own
/// unconditional rejection would otherwise fire, so `false` here is
/// always a no-op — this function can only ever turn a
/// previously-rejected call into an accepted one, never the reverse.
/// Zero false-positive risk on any currently-compiling program.
pub(crate) fn resolves_pure_at_call_site(
    callee_name: &str,
    call_arguments: &[Node],
    statements: &[Spanned<Node>],
    fn_effects: &HashMap<String, EffectSet>,
    caller_body: &Node,
) -> bool {
    let Some(Node::Function {
        parameters, body, ..
    }) = statements
        .iter()
        .map(|s| &s.node)
        .find(|n| matches!(n, Node::Function { name, .. } if name == callee_name))
    else {
        return false;
    };

    let invoked = invoked_callback_params(parameters, body);
    let effect_var_params: HashSet<&str> = parameters
        .iter()
        .filter(|(ty, name)| {
            is_function_type(ty) && has_effect_arrow(ty) && invoked.contains(name.as_str())
        })
        .map(|(_, name)| name.as_str())
        .collect();
    if effect_var_params.is_empty() {
        return false;
    }

    for (idx, (_ty, pname)) in parameters.iter().enumerate() {
        if !effect_var_params.contains(pname.as_str()) {
            continue;
        }
        match call_arguments.get(idx) {
            Some(Node::Identifier { name: arg_name, .. }) => {
                // RES-3933 A-E7 follow-up (#4097): trace a local
                // variable argument back to its initializer (see
                // [`resolve_local_alias`]) before giving up on it as
                // unresolvable.
                let resolved_name = if fn_effects.contains_key(arg_name) {
                    Some(arg_name.clone())
                } else {
                    resolve_local_alias(arg_name, caller_body)
                };
                match resolved_name.and_then(|n| fn_effects.get(&n).copied()) {
                    Some(effects) if effects.pure => {}
                    _ => return false,
                }
            }
            // RES-3933 A-E7: an inline lambda literal argument is
            // only rescued when it carries an *explicit* `pure
            // fn(...)` annotation. An unannotated bare `fn(...)`
            // literal (`explicit_effect: None`) can't be proven pure
            // here — its body isn't inspected by this pass — so it's
            // treated the same as any other unresolvable argument.
            Some(Node::FunctionLiteral {
                explicit_effect: Some(effects),
                ..
            }) if effects.pure => {}
            _ => return false,
        }
    }

    body_pure_modulo(body, &effect_var_params, fn_effects, parameters)
}

/// RES-3933 A-E7 follow-up (#4097): traces a plain identifier `var`
/// back through a *local* `let`-binding in `scope` to the name of the
/// value it was initialized with, e.g. resolving `g` to `"add1"` for
/// `let g = add1; run(g, x);`.
///
/// Returns `Some` only when the trace is unambiguous and sound:
///
/// - Exactly one `let var = <identifier>;` binding for `var` exists
///   anywhere in `scope` (a lambda/nested-fn body counts as part of
///   `scope` too, since `collect_local_bindings` doesn't special-case
///   `Node::Function`; that's conservative, not permissive — more
///   candidate bindings only make ambiguity *more* likely to be
///   detected, never less).
/// - `var` is never the target of a plain `Assignment` anywhere in
///   `scope` — if it were, the binding this call site actually sees
///   could differ from the initializer, so we can't be sure the alias
///   still holds. This is intentionally coarse (path-insensitive):
///   *any* reassignment anywhere disqualifies the trace, not just one
///   between the binding and this call site.
///
/// `scope` is normally the entire body of the enclosing top-level fn
/// (see `caller_body` threaded through `check_body_effects` /
/// `find_hof_call_violations`), which is exactly the set of statements
/// that could plausibly rebind `var`.
fn resolve_local_alias(var: &str, scope: &Node) -> Option<String> {
    let mut bindings: Vec<Option<String>> = Vec::new();
    let mut reassigned = false;
    collect_local_bindings(scope, var, &mut bindings, &mut reassigned);
    if reassigned || bindings.len() != 1 {
        return None;
    }
    bindings.into_iter().next().flatten()
}

/// Recursive walk collecting, for every `let var = ...;` in `node`,
/// `Some(name)` when the initializer is a plain identifier `name` or
/// `None` for anything else (so `bindings.len()` still counts
/// multiple bindings even when none are identifier-initialized), and
/// setting `reassigned` when `var` is ever the target of a plain
/// `Assignment`. Mirrors the node coverage of [`collect_invoked`].
fn collect_local_bindings(
    node: &Node,
    var: &str,
    bindings: &mut Vec<Option<String>>,
    reassigned: &mut bool,
) {
    match node {
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_local_bindings(s, var, bindings, reassigned);
            }
        }
        Node::LetStatement { name, value, .. } => {
            if name == var {
                bindings.push(match value.as_ref() {
                    Node::Identifier { name: id, .. } => Some(id.clone()),
                    _ => None,
                });
            }
            collect_local_bindings(value, var, bindings, reassigned);
        }
        Node::StaticLet { value, .. } => collect_local_bindings(value, var, bindings, reassigned),
        Node::ReturnStatement { value: Some(v), .. } => {
            collect_local_bindings(v, var, bindings, reassigned)
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_local_bindings(condition, var, bindings, reassigned);
            collect_local_bindings(consequence, var, bindings, reassigned);
            if let Some(a) = alternative {
                collect_local_bindings(a, var, bindings, reassigned);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_local_bindings(condition, var, bindings, reassigned);
            collect_local_bindings(body, var, bindings, reassigned);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_local_bindings(iterable, var, bindings, reassigned);
            collect_local_bindings(body, var, bindings, reassigned);
        }
        Node::Assert { condition, .. } | Node::Assume { condition, .. } => {
            collect_local_bindings(condition, var, bindings, reassigned);
        }
        Node::LiveBlock { body, .. } => collect_local_bindings(body, var, bindings, reassigned),
        Node::InfixExpression { left, right, .. } => {
            collect_local_bindings(left, var, bindings, reassigned);
            collect_local_bindings(right, var, bindings, reassigned);
        }
        Node::PrefixExpression { right, .. } => {
            collect_local_bindings(right, var, bindings, reassigned)
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            collect_local_bindings(function, var, bindings, reassigned);
            for a in arguments {
                collect_local_bindings(a, var, bindings, reassigned);
            }
        }
        Node::FieldAccess { target, .. } => {
            collect_local_bindings(target, var, bindings, reassigned)
        }
        Node::FieldAssignment { target, value, .. } => {
            collect_local_bindings(target, var, bindings, reassigned);
            collect_local_bindings(value, var, bindings, reassigned);
        }
        Node::Assignment { name, value, .. } => {
            if name == var {
                *reassigned = true;
            }
            collect_local_bindings(value, var, bindings, reassigned);
        }
        Node::IndexExpression { target, index, .. } => {
            collect_local_bindings(target, var, bindings, reassigned);
            collect_local_bindings(index, var, bindings, reassigned);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            collect_local_bindings(target, var, bindings, reassigned);
            collect_local_bindings(index, var, bindings, reassigned);
            collect_local_bindings(value, var, bindings, reassigned);
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                collect_local_bindings(i, var, bindings, reassigned);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(b) = base {
                collect_local_bindings(b, var, bindings, reassigned);
            }
            for (_, v) in fields {
                collect_local_bindings(v, var, bindings, reassigned);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_local_bindings(scrutinee, var, bindings, reassigned);
            for (_pat, guard, arm_body) in arms {
                if let Some(g) = guard {
                    collect_local_bindings(g, var, bindings, reassigned);
                }
                collect_local_bindings(arm_body, var, bindings, reassigned);
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            collect_local_bindings(expr, var, bindings, reassigned)
        }
        Node::TryExpression { expr, .. } => collect_local_bindings(expr, var, bindings, reassigned),
        Node::OptionalChain { object, access, .. } => {
            collect_local_bindings(object, var, bindings, reassigned);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    collect_local_bindings(a, var, bindings, reassigned);
                }
            }
        }
        Node::Function { body, .. } => collect_local_bindings(body, var, bindings, reassigned),
        _ => {}
    }
}

/// RES-4123: named effect-variable unification across a HOF's
/// invoked, function-typed parameters. See the module doc's "Named
/// effect-variable unification" section.
///
/// For each top-level fn, groups its invoked (see
/// [`invoked_callback_params`]) function-typed parameters by their
/// `-e->` letter (see [`effect_arrow_letter`]) and, for every group of
/// two or more parameters sharing a letter, checks every call site of
/// that fn anywhere in the program: if two same-letter parameters
/// resolve — at that specific call site — to arguments with
/// different provable purity, that's a proven violation (no single
/// effect could satisfy both), and is rejected. Runs for every
/// top-level fn regardless of its own declared effect, since the
/// requirement is intrinsic to the callee's signature.
pub(crate) fn check_effect_var_unification(
    statements: &[Spanned<Node>],
    fn_effects: &HashMap<String, EffectSet>,
    source_path: &str,
) -> Result<(), String> {
    for stmt in statements {
        let Node::Function {
            name: hof_name,
            parameters,
            body,
            ..
        } = &stmt.node
        else {
            continue;
        };
        let invoked = invoked_callback_params(parameters, body);
        if invoked.is_empty() {
            continue;
        }

        let mut groups: std::collections::BTreeMap<char, Vec<(usize, &str)>> =
            std::collections::BTreeMap::new();
        for (idx, (ty, pname)) in parameters.iter().enumerate() {
            if !is_function_type(ty) || !invoked.contains(pname.as_str()) {
                continue;
            }
            if let Some(letter) = effect_arrow_letter(ty) {
                groups
                    .entry(letter)
                    .or_default()
                    .push((idx, pname.as_str()));
            }
        }

        for (letter, members) in groups.iter().filter(|(_, m)| m.len() >= 2) {
            check_unification_at_call_sites(
                statements,
                hof_name,
                *letter,
                members,
                fn_effects,
                source_path,
            )?;
        }
    }
    Ok(())
}

/// Scans every call site of `hof_name` anywhere in `statements` and
/// rejects the first one where two of `members` (parameter `(idx,
/// name)` pairs sharing effect-variable `letter`) resolve to arguments
/// with different provable purity.
fn check_unification_at_call_sites(
    statements: &[Spanned<Node>],
    hof_name: &str,
    letter: char,
    members: &[(usize, &str)],
    fn_effects: &HashMap<String, EffectSet>,
    source_path: &str,
) -> Result<(), String> {
    for stmt in statements {
        let mut sites: Vec<(crate::span::Span, &[Node])> = Vec::new();
        collect_hof_call_sites(&stmt.node, hof_name, &mut sites);
        for (span, args) in sites {
            let mut resolved: Vec<(&str, EffectSet)> = Vec::new();
            for &(idx, pname) in members {
                if let Some(arg) = args.get(idx)
                    && let Some(effects) = resolve_arg_effect(arg, fn_effects, &stmt.node)
                {
                    resolved.push((pname, effects));
                }
            }
            for i in 0..resolved.len() {
                for j in (i + 1)..resolved.len() {
                    let (p1, e1) = resolved[i];
                    let (p2, e2) = resolved[j];
                    if e1.pure != e2.pure {
                        let (line, col) = (span.start.line, span.start.column);
                        let msg = format!(
                            "cannot unify effect variable '{letter}' at call to `{hof_name}`: \
                             parameter `{p1}` resolves to {} effect but parameter `{p2}` \
                             resolves to {} effect",
                            e1.tag(),
                            e2.tag()
                        );
                        return Err(if line == 0 {
                            msg
                        } else {
                            format!("{source_path}:{line}:{col}: {msg}")
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// Resolves a call-argument `Node` to a concrete [`EffectSet`] when
/// provable: a plain identifier naming a top-level fn (or a local
/// variable unambiguously aliasing one, see [`resolve_local_alias`]),
/// or an inline lambda literal carrying an *explicit* `pure`/`io`
/// annotation (RES-4123, mirrors the resolution already used by
/// [`find_hof_call_violations`] and [`resolves_pure_at_call_site`]).
/// Returns `None` for anything else — an unresolvable argument is
/// never compared against a sibling, preserving "when uncertain,
/// accept".
fn resolve_arg_effect(
    arg: &Node,
    fn_effects: &HashMap<String, EffectSet>,
    caller_body: &Node,
) -> Option<EffectSet> {
    match arg {
        Node::Identifier { name, .. } => {
            let resolved_name = if fn_effects.contains_key(name) {
                Some(name.clone())
            } else {
                resolve_local_alias(name, caller_body)
            };
            resolved_name.and_then(|n| fn_effects.get(&n).copied())
        }
        Node::FunctionLiteral {
            explicit_effect: Some(effects),
            ..
        } => Some(*effects),
        _ => None,
    }
}

/// Collects every `(span, arguments)` pair for `CallExpression` nodes
/// that call `hof_name` anywhere under `node`. Mirrors the node
/// coverage of [`find_hof_call_violations`] exactly (see that
/// function for why each variant is included), but *collects* every
/// matching call site instead of stopping at the first proven
/// violation for a single parameter index — [`check_unification_at_call_sites`]
/// needs to inspect multiple parameter indices per site.
fn collect_hof_call_sites<'a>(
    node: &'a Node,
    hof_name: &str,
    out: &mut Vec<(crate::span::Span, &'a [Node])>,
) {
    match node {
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_hof_call_sites(s, hof_name, out);
            }
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            collect_hof_call_sites(value, hof_name, out);
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            collect_hof_call_sites(v, hof_name, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_hof_call_sites(condition, hof_name, out);
            collect_hof_call_sites(consequence, hof_name, out);
            if let Some(a) = alternative {
                collect_hof_call_sites(a, hof_name, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_hof_call_sites(condition, hof_name, out);
            collect_hof_call_sites(body, hof_name, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_hof_call_sites(iterable, hof_name, out);
            collect_hof_call_sites(body, hof_name, out);
        }
        Node::Assert { condition, .. } | Node::Assume { condition, .. } => {
            collect_hof_call_sites(condition, hof_name, out);
        }
        Node::LiveBlock { body, .. } => collect_hof_call_sites(body, hof_name, out),
        Node::InfixExpression { left, right, .. } => {
            collect_hof_call_sites(left, hof_name, out);
            collect_hof_call_sites(right, hof_name, out);
        }
        Node::PrefixExpression { right, .. } => collect_hof_call_sites(right, hof_name, out),
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            if let Node::Identifier { name, .. } = function.as_ref()
                && name == hof_name
            {
                out.push((*span, arguments.as_slice()));
            }
            collect_hof_call_sites(function, hof_name, out);
            for a in arguments {
                collect_hof_call_sites(a, hof_name, out);
            }
        }
        Node::FieldAccess { target, .. } => collect_hof_call_sites(target, hof_name, out),
        Node::FieldAssignment { target, value, .. } => {
            collect_hof_call_sites(target, hof_name, out);
            collect_hof_call_sites(value, hof_name, out);
        }
        Node::Assignment { value, .. } => collect_hof_call_sites(value, hof_name, out),
        Node::IndexExpression { target, index, .. } => {
            collect_hof_call_sites(target, hof_name, out);
            collect_hof_call_sites(index, hof_name, out);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            collect_hof_call_sites(target, hof_name, out);
            collect_hof_call_sites(index, hof_name, out);
            collect_hof_call_sites(value, hof_name, out);
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                collect_hof_call_sites(i, hof_name, out);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(b) = base {
                collect_hof_call_sites(b, hof_name, out);
            }
            for (_, v) in fields {
                collect_hof_call_sites(v, hof_name, out);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_hof_call_sites(scrutinee, hof_name, out);
            for (_pat, guard, arm_body) in arms {
                if let Some(g) = guard {
                    collect_hof_call_sites(g, hof_name, out);
                }
                collect_hof_call_sites(arm_body, hof_name, out);
            }
        }
        Node::ExpressionStatement { expr, .. } => collect_hof_call_sites(expr, hof_name, out),
        Node::TryExpression { expr, .. } => collect_hof_call_sites(expr, hof_name, out),
        Node::OptionalChain { object, access, .. } => {
            collect_hof_call_sites(object, hof_name, out);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    collect_hof_call_sites(a, hof_name, out);
                }
            }
        }
        Node::Function { body, .. } => collect_hof_call_sites(body, hof_name, out),
        _ => {}
    }
}

/// Extracts the single-letter effect-variable name from a `fn(...)`
/// parameter type-annotation string carrying the RES-193 `-e->`
/// arrow marker (e.g. `"fn(int) -e-> int"` → `Some('e')`), or `None`
/// when `ty` has no effect arrow (plain `-> R`) — see
/// [`has_effect_arrow`], which this mirrors but returns the letter
/// itself rather than a bool.
fn effect_arrow_letter(ty: &str) -> Option<char> {
    ty.split_whitespace().find_map(|tok| {
        if tok.len() >= 4 && tok.starts_with('-') && tok.ends_with("->") {
            let mut inner = tok[1..tok.len() - 2].chars();
            let letter = inner.next()?;
            if inner.next().is_none() {
                Some(letter)
            } else {
                None
            }
        } else {
            None
        }
    })
}

/// True when `ty` (a `fn(...)`-shaped type-annotation string) carries
/// the RES-193 `-e->` effect-arrow marker, i.e. was parsed from
/// `fn(...) -e-> R` rather than the plain `fn(...) -> R` form. The
/// arrow renders as a single letter sandwiched between two `-`s,
/// ending in `>` (`"-e->"`) — see `parse_type_annotation`'s
/// `Token::Function` arm in `lib.rs`, which is the sole producer of
/// this string shape.
fn has_effect_arrow(ty: &str) -> bool {
    ty.split_whitespace().any(|tok| {
        tok.len() >= 4
            && tok.starts_with('-')
            && tok.ends_with("->")
            && tok[1..tok.len() - 2].chars().count() == 1
    })
}

/// Recursive walk mirroring `typechecker::check_body_effects`'s
/// pure-body rules, used to probe a *candidate* effect-polymorphic
/// HOF's body rather than an explicitly `pure fn`. Two differences
/// suit that purpose:
///
/// - A call whose callee is one of `deferred` (the HOF's own invoked
///   effect-variable parameters) is always allowed — the caller
///   ([`resolves_pure_at_call_site`]) has already separately proven
///   every actual argument bound to one of those parameters is
///   provably pure at this specific call site.
/// - Everything else must independently satisfy the exact same rules
///   `check_body_effects` enforces: user fns must be `pure`-declared,
///   builtins must be on the pure-by-default list, and consuming one
///   of `fn_params` that's `linear` is rejected (mirrors
///   `check_body_effects`'s `linear_params` handling, applied here to
///   the *candidate*'s own parameters).
///
/// Returns `bool` rather than `Result<_, String>` — the caller only
/// needs a yes/no answer; on `false` it silently falls back to the
/// existing rejection diagnostic.
fn body_pure_modulo(
    node: &Node,
    deferred: &HashSet<&str>,
    fn_effects: &HashMap<String, EffectSet>,
    fn_params: &[(String, String)],
) -> bool {
    match node {
        Node::Block { stmts, .. } => stmts
            .iter()
            .all(|s| body_pure_modulo(s, deferred, fn_effects, fn_params)),
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            body_pure_modulo(value, deferred, fn_effects, fn_params)
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            body_pure_modulo(v, deferred, fn_effects, fn_params)
        }
        Node::ReturnStatement { value: None, .. } => true,
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            body_pure_modulo(condition, deferred, fn_effects, fn_params)
                && body_pure_modulo(consequence, deferred, fn_effects, fn_params)
                && alternative
                    .as_ref()
                    .is_none_or(|a| body_pure_modulo(a, deferred, fn_effects, fn_params))
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            body_pure_modulo(condition, deferred, fn_effects, fn_params)
                && body_pure_modulo(body, deferred, fn_effects, fn_params)
        }
        Node::ForInStatement { iterable, body, .. } => {
            body_pure_modulo(iterable, deferred, fn_effects, fn_params)
                && body_pure_modulo(body, deferred, fn_effects, fn_params)
        }
        Node::Assert { condition, .. } | Node::Assume { condition, .. } => {
            body_pure_modulo(condition, deferred, fn_effects, fn_params)
        }
        Node::LiveBlock { body, .. } => body_pure_modulo(body, deferred, fn_effects, fn_params),
        Node::InfixExpression { left, right, .. } => {
            body_pure_modulo(left, deferred, fn_effects, fn_params)
                && body_pure_modulo(right, deferred, fn_effects, fn_params)
        }
        Node::PrefixExpression { right, .. } => {
            body_pure_modulo(right, deferred, fn_effects, fn_params)
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if !arguments
                .iter()
                .all(|a| body_pure_modulo(a, deferred, fn_effects, fn_params))
            {
                return false;
            }
            for a in arguments {
                if let Node::Identifier { name: arg_name, .. } = a
                    && fn_params.iter().any(|(param_ty, param_name)| {
                        arg_name == param_name && crate::linear::is_linear(param_ty)
                    })
                {
                    return false;
                }
            }
            if let Node::Identifier { name: callee, .. } = function.as_ref() {
                if deferred.contains(callee.as_str()) {
                    return true;
                }
                if let Some(callee_effects) = fn_effects.get(callee) {
                    return callee_effects.pure;
                }
                if crate::typechecker::IMPURE_BUILTINS.contains(&callee.as_str()) {
                    return false;
                }
                return crate::typechecker::is_known_pure_builtin(callee);
            }
            false
        }
        Node::FieldAccess { target, .. } => {
            body_pure_modulo(target, deferred, fn_effects, fn_params)
        }
        Node::FieldAssignment { target, value, .. } => {
            body_pure_modulo(target, deferred, fn_effects, fn_params)
                && body_pure_modulo(value, deferred, fn_effects, fn_params)
        }
        Node::Assignment { value, .. } => body_pure_modulo(value, deferred, fn_effects, fn_params),
        Node::IndexExpression { target, index, .. } => {
            body_pure_modulo(target, deferred, fn_effects, fn_params)
                && body_pure_modulo(index, deferred, fn_effects, fn_params)
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            body_pure_modulo(target, deferred, fn_effects, fn_params)
                && body_pure_modulo(index, deferred, fn_effects, fn_params)
                && body_pure_modulo(value, deferred, fn_effects, fn_params)
        }
        Node::ArrayLiteral { items, .. } => items
            .iter()
            .all(|i| body_pure_modulo(i, deferred, fn_effects, fn_params)),
        Node::StructLiteral { fields, base, .. } => {
            base.as_ref()
                .is_none_or(|b| body_pure_modulo(b, deferred, fn_effects, fn_params))
                && fields
                    .iter()
                    .all(|(_, v)| body_pure_modulo(v, deferred, fn_effects, fn_params))
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            body_pure_modulo(scrutinee, deferred, fn_effects, fn_params)
                && arms.iter().all(|(_pat, guard, arm_body)| {
                    guard
                        .as_ref()
                        .is_none_or(|g| body_pure_modulo(g, deferred, fn_effects, fn_params))
                        && body_pure_modulo(arm_body, deferred, fn_effects, fn_params)
                })
        }
        Node::ExpressionStatement { expr, .. } => {
            body_pure_modulo(expr, deferred, fn_effects, fn_params)
        }
        Node::TryExpression { expr, .. } => body_pure_modulo(expr, deferred, fn_effects, fn_params),
        Node::OptionalChain { object, access, .. } => {
            body_pure_modulo(object, deferred, fn_effects, fn_params)
                && match access {
                    crate::ChainAccess::Method(_, args) => args
                        .iter()
                        .all(|a| body_pure_modulo(a, deferred, fn_effects, fn_params)),
                    _ => true,
                }
        }
        Node::Function { body, .. } => body_pure_modulo(body, deferred, fn_effects, fn_params),
        _ => true,
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

    /// Finds the arguments of the first call to `callee` anywhere in
    /// `statements` — enough to drive [`resolves_pure_at_call_site`]
    /// directly in a unit test without hand-building `Node::Identifier`
    /// literals.
    fn find_call_args<'a>(statements: &'a [Spanned<Node>], callee: &str) -> &'a [Node] {
        fn walk<'a>(node: &'a Node, callee: &str) -> Option<&'a [Node]> {
            match node {
                Node::Block { stmts, .. } => stmts.iter().find_map(|s| walk(s, callee)),
                Node::ReturnStatement { value: Some(v), .. } => walk(v, callee),
                Node::ExpressionStatement { expr, .. } => walk(expr, callee),
                Node::Function { body, .. } => walk(body, callee),
                Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
                    walk(value, callee)
                }
                Node::CallExpression {
                    function,
                    arguments,
                    ..
                } => {
                    if let Node::Identifier { name, .. } = function.as_ref()
                        && name == callee
                    {
                        return Some(arguments.as_slice());
                    }
                    None
                }
                _ => None,
            }
        }
        statements
            .iter()
            .find_map(|s| walk(&s.node, callee))
            .unwrap_or_else(|| panic!("no call to `{callee}` found"))
    }

    /// Finds the body of the top-level fn named `name` — used as the
    /// `caller_body` argument to [`resolves_pure_at_call_site`] and
    /// [`resolve_local_alias`] in tests, since every test fixture below
    /// calls the HOF under test from inside `fn caller() { ... }`.
    fn find_fn_body<'a>(statements: &'a [Spanned<Node>], name: &str) -> &'a Node {
        statements
            .iter()
            .find_map(|s| match &s.node {
                Node::Function { name: n, body, .. } if n == name => Some(body.as_ref()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no fn `{name}` found"))
    }

    #[test]
    fn has_effect_arrow_detects_res193_marker() {
        assert!(has_effect_arrow("fn(int) -e-> int"));
        assert!(!has_effect_arrow("fn(int) -> int"));
        assert!(!has_effect_arrow("int"));
    }

    #[test]
    fn polymorphic_hof_accepts_pure_callback_at_call_site() {
        // `run` never declares `pure fn` — its effect is genuinely
        // polymorphic via the `-e->` marker on `f`. Calling it with a
        // provably-pure callback must be provable pure at this site.
        let src = "fn run(fn(int) -e-> int f, int x) -> int { return f(x); }\n\
                   pure fn add1(int x) -> int { return x + 1; }\n\
                   fn caller() -> int { return run(add1, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let args = find_call_args(&s, "run");
        assert!(
            resolves_pure_at_call_site("run", args, &s, &fe, find_fn_body(&s, "caller")),
            "pure callback into an effect-polymorphic HOF must resolve pure"
        );
    }

    #[test]
    fn polymorphic_hof_rejects_io_callback_at_call_site() {
        let src = "fn run(fn(int) -e-> int f, int x) -> int { return f(x); }\n\
                   io fn noisy(int x) -> int { println(x); return x; }\n\
                   fn caller() -> int { return run(noisy, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let args = find_call_args(&s, "run");
        assert!(
            !resolves_pure_at_call_site("run", args, &s, &fe, find_fn_body(&s, "caller")),
            "io callback into an effect-polymorphic HOF must not resolve pure"
        );
    }

    #[test]
    fn polymorphic_hof_with_extra_impurity_is_never_rescued() {
        // `run` does something impure (`println`) *unconditionally*,
        // regardless of what `f` turns out to be — this must never be
        // treated as pure even when the callback itself is pure,
        // otherwise a `pure` caller could transitively observe I/O.
        let src = "fn run(fn(int) -e-> int f, int x) -> int { \
                   println(\"go\"); return f(x); }\n\
                   pure fn add1(int x) -> int { return x + 1; }\n\
                   fn caller() -> int { return run(add1, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let args = find_call_args(&s, "run");
        assert!(
            !resolves_pure_at_call_site("run", args, &s, &fe, find_fn_body(&s, "caller")),
            "a HOF with unconditional impurity beyond its callback must never be rescued"
        );
    }

    #[test]
    fn hof_without_effect_arrow_marker_is_not_rescued() {
        // No `-e->` marker on `f` at all — this is the monomorphic
        // case `check`/`check_call_sites` already handles; the
        // polymorphism probe must not also fire for it.
        let src = "fn run(fn(int) -> int f, int x) -> int { return f(x); }\n\
                   pure fn add1(int x) -> int { return x + 1; }\n\
                   fn caller() -> int { return run(add1, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let args = find_call_args(&s, "run");
        assert!(!resolves_pure_at_call_site(
            "run",
            args,
            &s,
            &fe,
            find_fn_body(&s, "caller")
        ));
    }

    #[test]
    fn hof_with_unresolvable_argument_is_not_rescued() {
        // The argument bound to `f` is a local parameter, not a
        // plain top-level fn name — its effect can't be proven, so
        // per "when uncertain, don't rescue" this must stay
        // unrescued (leaving the existing rejection in place, not a
        // regression since the call was already rejected before this
        // pass existed).
        let src = "fn run(fn(int) -e-> int f, int x) -> int { return f(x); }\n\
                   fn caller(fn(int) -> int cb) -> int { return run(cb, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let args = find_call_args(&s, "run");
        assert!(!resolves_pure_at_call_site(
            "run",
            args,
            &s,
            &fe,
            find_fn_body(&s, "caller")
        ));
    }

    // ---- RES-3933 A-E7 follow-up (#4097): local-variable callback tracing ----

    #[test]
    fn monomorphic_hof_traces_local_alias_of_io_callback() {
        // `g` is a local variable bound to `noisy` and never
        // reassigned — `check` must trace it back to `noisy` and
        // reject, the same as if `noisy` were passed directly.
        let src = "io fn noisy(int x) -> int { println(x); return x; }\n\
                   pure fn run(fn(int) -> int f, int x) -> int { return f(x); }\n\
                   fn caller() -> int { let g = noisy; return run(g, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let err = check(&s, &fe, "<t>")
            .expect_err("io callback aliased through a local var must still be rejected");
        assert!(
            err.contains("cannot pass io callback `noisy` to pure higher-order function `run`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn monomorphic_hof_traces_local_alias_of_pure_callback() {
        let src = "pure fn add1(int x) -> int { return x + 1; }\n\
                   pure fn run(fn(int) -> int f, int x) -> int { return f(x); }\n\
                   fn caller() -> int { let g = add1; return run(g, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        check(&s, &fe, "<t>")
            .expect("pure callback aliased through a local var must still be accepted");
    }

    #[test]
    fn monomorphic_hof_does_not_trace_reassigned_local_alias() {
        // `g` is reassigned somewhere in `caller` — even though the
        // reassignment here happens to be irrelevant to this call
        // site, the pass is path-insensitive and must not trust the
        // trace once any reassignment exists anywhere in the fn.
        let src = "io fn noisy(int x) -> int { println(x); return x; }\n\
                   pure fn add1(int x) -> int { return x + 1; }\n\
                   pure fn run(fn(int) -> int f, int x) -> int { return f(x); }\n\
                   fn caller() -> int {\n\
                   \x20   let g = noisy;\n\
                   \x20   let r = run(g, 5);\n\
                   \x20   g = add1;\n\
                   \x20   return r;\n\
                   }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        // Unresolvable once reassigned anywhere — "when uncertain,
        // accept" applies, so this must stay permissive rather than
        // erroring on a stale/ambiguous trace.
        check(&s, &fe, "<t>").expect("reassigned local alias must not be trusted either way");
    }

    #[test]
    fn monomorphic_hof_does_not_trace_ambiguous_local_alias() {
        // Two distinct `let g = ...;` bindings for the same name in
        // the same fn (shadowing) — ambiguous, must not be traced.
        let src = "io fn noisy(int x) -> int { println(x); return x; }\n\
                   pure fn add1(int x) -> int { return x + 1; }\n\
                   pure fn run(fn(int) -> int f, int x) -> int { return f(x); }\n\
                   fn caller() -> int {\n\
                   \x20   let g = add1;\n\
                   \x20   if true { let g = noisy; }\n\
                   \x20   return run(g, 5);\n\
                   }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        check(&s, &fe, "<t>").expect("ambiguous (shadowed) local alias must not be trusted");
    }

    #[test]
    fn polymorphic_hof_traces_local_alias_of_pure_callback_at_call_site() {
        let src = "fn run(fn(int) -e-> int f, int x) -> int { return f(x); }\n\
                   pure fn add1(int x) -> int { return x + 1; }\n\
                   fn caller() -> int { let g = add1; return run(g, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let args = find_call_args(&s, "run");
        assert!(
            resolves_pure_at_call_site("run", args, &s, &fe, find_fn_body(&s, "caller")),
            "pure callback aliased through a local var must resolve pure at this call site"
        );
    }

    #[test]
    fn polymorphic_hof_does_not_trace_reassigned_local_alias_at_call_site() {
        let src = "fn run(fn(int) -e-> int f, int x) -> int { return f(x); }\n\
                   pure fn add1(int x) -> int { return x + 1; }\n\
                   io fn noisy(int x) -> int { println(x); return x; }\n\
                   fn caller() -> int {\n\
                   \x20   let g = add1;\n\
                   \x20   let r = run(g, 5);\n\
                   \x20   g = noisy;\n\
                   \x20   return r;\n\
                   }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let args = find_call_args(&s, "run");
        assert!(
            !resolves_pure_at_call_site("run", args, &s, &fe, find_fn_body(&s, "caller")),
            "reassigned local alias must not be rescued"
        );
    }

    #[test]
    fn resolve_local_alias_unit_cases() {
        let src = "fn caller() -> int {\n\
                   \x20   let g = add1;\n\
                   \x20   return g;\n\
                   }\n";
        let s = stmts(src);
        let body = find_fn_body(&s, "caller");
        assert_eq!(resolve_local_alias("g", body), Some("add1".to_string()));
        assert_eq!(resolve_local_alias("nonexistent", body), None);
    }

    // ---------- RES-4123: named effect-variable unification ----------

    #[test]
    fn effect_arrow_letter_extracts_single_letter() {
        assert_eq!(effect_arrow_letter("fn(int) -e-> int"), Some('e'));
        assert_eq!(effect_arrow_letter("fn(int) -f-> int"), Some('f'));
        assert_eq!(effect_arrow_letter("fn(int) -> int"), None);
        assert_eq!(effect_arrow_letter("int"), None);
    }

    #[test]
    fn unification_rejects_mismatched_same_letter_params() {
        // `f` and `g` both share effect variable `e` — binding one to a
        // provably-pure fn and the other to a provably-io fn at the
        // same call site can never be satisfied by a single effect.
        let src = "pure fn add1(int x) -> int { return x + 1; }\n\
                   io fn noisy(int x) -> int { println(x); return x; }\n\
                   fn combine(fn(int) -e-> int f, fn(int) -e-> int g, int x) -> int {\n\
                   \x20   return f(x) + g(x);\n\
                   }\n\
                   fn caller() -> int { return combine(add1, noisy, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let err = check_effect_var_unification(&s, &fe, "<t>").unwrap_err();
        assert!(err.contains("cannot unify effect variable 'e'"), "{err}");
        assert!(err.contains('f'), "{err}");
        assert!(err.contains('g'), "{err}");
    }

    #[test]
    fn unification_accepts_matching_same_letter_params_both_pure() {
        let src = "pure fn add1(int x) -> int { return x + 1; }\n\
                   pure fn double(int x) -> int { return x * 2; }\n\
                   fn combine(fn(int) -e-> int f, fn(int) -e-> int g, int x) -> int {\n\
                   \x20   return f(x) + g(x);\n\
                   }\n\
                   fn caller() -> int { return combine(add1, double, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        check_effect_var_unification(&s, &fe, "<t>")
            .expect("both params provably pure — must unify cleanly");
    }

    #[test]
    fn unification_accepts_matching_same_letter_params_both_io() {
        let src = "io fn noisy1(int x) -> int { println(x); return x; }\n\
                   io fn noisy2(int x) -> int { println(x); return x; }\n\
                   fn combine(fn(int) -e-> int f, fn(int) -e-> int g, int x) -> int {\n\
                   \x20   return f(x) + g(x);\n\
                   }\n\
                   fn caller() -> int { return combine(noisy1, noisy2, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        check_effect_var_unification(&s, &fe, "<t>")
            .expect("both params provably io — must unify cleanly");
    }

    #[test]
    fn unification_leaves_distinct_letters_independent() {
        // `f` is `-e->`, `g` is `-f->` — distinct effect variables, so
        // one being pure and the other io is not a unification error.
        let src = "pure fn add1(int x) -> int { return x + 1; }\n\
                   io fn noisy(int x) -> int { println(x); return x; }\n\
                   fn combine(fn(int) -e-> int f, fn(int) -f-> int g, int x) -> int {\n\
                   \x20   return f(x) + g(x);\n\
                   }\n\
                   fn caller() -> int { return combine(add1, noisy, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        check_effect_var_unification(&s, &fe, "<t>")
            .expect("distinct effect-variable letters must not be unified against each other");
    }

    #[test]
    fn unification_is_conservative_when_one_argument_is_unresolvable() {
        // `g` is a fn-typed parameter of `caller` itself — its effect
        // can't be proven, so "when uncertain, accept" applies even
        // though `f` resolves to `pure`.
        let src = "pure fn add1(int x) -> int { return x + 1; }\n\
                   fn combine(fn(int) -e-> int f, fn(int) -e-> int g, int x) -> int {\n\
                   \x20   return f(x) + g(x);\n\
                   }\n\
                   fn caller(fn(int) -> int cb) -> int { return combine(add1, cb, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        check_effect_var_unification(&s, &fe, "<t>")
            .expect("unresolvable sibling argument must not be flagged");
    }

    #[test]
    fn unification_rejects_via_local_alias_and_lambda_literal() {
        // Same-letter mismatch proven through a local-variable alias
        // on one side and an explicit inline lambda literal on the
        // other — both resolution paths must feed into unification.
        let src = "pure fn add1(int x) -> int { return x + 1; }\n\
                   fn combine(fn(int) -e-> int f, fn(int) -e-> int g, int x) -> int {\n\
                   \x20   return f(x) + g(x);\n\
                   }\n\
                   fn caller() -> int {\n\
                   \x20   let h = add1;\n\
                   \x20   return combine(h, io fn(int a) -> int { println(a); return a; }, 5);\n\
                   }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let err = check_effect_var_unification(&s, &fe, "<t>").unwrap_err();
        assert!(err.contains("cannot unify effect variable 'e'"), "{err}");
    }

    #[test]
    fn unification_check_wired_into_check() {
        // `check` (the top-level entry point) must run the
        // unification pass too, not just the pure-HOF rescue path.
        let src = "pure fn add1(int x) -> int { return x + 1; }\n\
                   io fn noisy(int x) -> int { println(x); return x; }\n\
                   fn combine(fn(int) -e-> int f, fn(int) -e-> int g, int x) -> int {\n\
                   \x20   return f(x) + g(x);\n\
                   }\n\
                   fn caller() -> int { return combine(add1, noisy, 5); }\n";
        let s = stmts(src);
        let fe = effects_of(&s);
        let err = check(&s, &fe, "<t>").unwrap_err();
        assert!(err.contains("cannot unify effect variable 'e'"), "{err}");
    }
}
