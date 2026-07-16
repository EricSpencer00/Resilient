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
//! ## Deferred (tracked as a follow-up issue, see the PR body)
//!
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
//! - A HOF's *own* return-type effect arrow (`fn run(...) -e-> int`)
//!   is parsed but the letter is discarded (RES-193 only preserves it
//!   on function-typed *parameter* types); [`resolves_pure_at_call_site`]
//!   sidesteps this by inferring polymorphism structurally (any
//!   invoked `-e->`-marked parameter) rather than requiring the HOF's
//!   own return type to declare a matching variable name. A follow-up
//!   could add named-variable unification across multiple
//!   effect-variable parameters that must agree with each other.
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
        let Some(Node::Identifier { name: arg_name, .. }) = call_arguments.get(idx) else {
            return false;
        };
        match fn_effects.get(arg_name) {
            Some(effects) if effects.pure => {}
            _ => return false,
        }
    }

    body_pure_modulo(body, &effect_var_params, fn_effects, parameters)
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
            resolves_pure_at_call_site("run", args, &s, &fe),
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
            !resolves_pure_at_call_site("run", args, &s, &fe),
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
            !resolves_pure_at_call_site("run", args, &s, &fe),
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
        assert!(!resolves_pure_at_call_site("run", args, &s, &fe));
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
        assert!(!resolves_pure_at_call_site("run", args, &s, &fe));
    }
}
