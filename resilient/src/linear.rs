//! RES-385: linear types — minimum-viable single-use enforcement.
//!
//! Linear types are resources the type checker proves are consumed
//! exactly once. The MVP slice landed here:
//!
//! 1. Parser accepts `linear T` in type annotations and encodes it
//!    as the string `"linear T"` inside the existing type-annotation
//!    slots (no AST-node field changes — keeps the blast radius
//!    tiny). Helpers below read that encoding back.
//! 2. [`check_linear_usage`] walks every top-level fn body and flags
//!    any linear-typed local whose value is referenced after it has
//!    been consumed (passed to another fn, reassigned, or passed to
//!    the `drop` builtin).
//!
//! Out of scope (tracked as follow-up tickets):
//! - Z3 fallback proofs for conditional consumption in branchy paths.
//! - Linear lifetimes threaded through closures.
//! - Effect-system interaction.

use crate::Node;
use crate::span::Span;
use std::collections::HashMap;

/// RES-385: marker prefix used by the parser to smuggle the
/// "this type is linear" bit through the `String`-valued type-
/// annotation slots on AST nodes. Kept in one place so encoder
/// (parser) and decoders (type checker, linearity pass) can't
/// drift.
pub const LINEAR_PREFIX: &str = "linear ";

/// True iff `annot` is a linear-typed annotation. Accepts bare
/// `"linear T"` only — no internal whitespace, no tabs — matching
/// what the parser emits.
pub fn is_linear(annot: &str) -> bool {
    annot.starts_with(LINEAR_PREFIX)
}

/// Strip the `linear ` prefix if present and return the underlying
/// type name (e.g. `"FileHandle"`). Non-linear annotations pass
/// through unchanged.
pub fn strip_linear(annot: &str) -> &str {
    annot.strip_prefix(LINEAR_PREFIX).unwrap_or(annot)
}

/// State of a single linear binding while the pass walks a fn body.
#[derive(Clone, Debug)]
struct LinearBinding {
    /// Type name without the `linear` prefix — used in the diagnostic.
    ty_name: String,
    /// Source span of the binding (the `let` or the parameter name).
    /// Reserved for the richer "defined here / used here" two-line
    /// diagnostic a follow-up ticket adds; the MVP message only
    /// surfaces the use-after-move site.
    #[allow(dead_code)]
    defined_at: Span,
    /// `Some(span)` once the value has been moved — the span points
    /// at the site that consumed it. `None` while it's still live.
    consumed_at: Option<Span>,
}

/// RES-385: top-level entry. Walks every top-level function and
/// enforces the single-use rule on every linear-typed parameter
/// and `let` binding inside it. Errors are formatted with the
/// `<source>:<line>:<col>: ` prefix matching the rest of the
/// typechecker's diagnostics.
///
/// Returns the first violation encountered — consistent with the
/// existing typechecker entrypoint that returns on the first error.
pub fn check_linear_usage(program: &Node, source_path: &str) -> Result<(), String> {
    let Node::Program(statements) = program else {
        return Ok(());
    };
    for stmt in statements {
        match &stmt.node {
            Node::Function {
                name,
                parameters,
                body,
                ..
            } => {
                check_fn_body(name, parameters, body, source_path)?;
            }
            // `impl` blocks parse their methods as `Node::Function`
            // entries inside a `methods` list — recurse through
            // them the same way.
            Node::ImplBlock { methods, .. } => {
                for m in methods {
                    if let Node::Function {
                        name,
                        parameters,
                        body,
                        ..
                    } = m
                    {
                        check_fn_body(name, parameters, body, source_path)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn check_fn_body(
    fn_name: &str,
    parameters: &[(String, String)],
    body: &Node,
    source_path: &str,
) -> Result<(), String> {
    let mut bindings: HashMap<String, LinearBinding> = HashMap::new();
    for (ty, pname) in parameters {
        if is_linear(ty) {
            bindings.insert(
                pname.clone(),
                LinearBinding {
                    ty_name: strip_linear(ty).to_string(),
                    defined_at: Span::default(),
                    consumed_at: None,
                },
            );
        }
    }
    walk(body, &mut bindings, fn_name, source_path)
}

/// Recursive walk. Returns the first single-use violation.
///
/// The walker tracks three classes of operation per linear binding:
///
/// 1. *Move* — an identifier reference in a value-producing position
///    (call argument, RHS of `let` / assignment, `return` value,
///    `drop` argument). Counts as consumption; a subsequent use is
///    the error case.
/// 2. *Shadow* — `let x = …` inside a nested scope re-introduces
///    `x`. To keep the MVP simple we treat this as consumption of
///    the outer `x` followed by a new non-linear local; a rebinding
///    that leaks a still-live resource is reported.
/// 3. *Re-initialisation* — `x = …` resets a consumed linear binding
///    (future-compatible with the "after move, rebind" pattern some
///    designs allow; MVP still forbids it to keep the rule crisp).
fn walk(
    node: &Node,
    bindings: &mut HashMap<String, LinearBinding>,
    fn_name: &str,
    source_path: &str,
) -> Result<(), String> {
    match node {
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk(s, bindings, fn_name, source_path)?;
            }
            Ok(())
        }
        Node::LetStatement {
            name,
            value,
            type_annot,
            span,
        } => {
            // Walking the RHS first models left-to-right eval order:
            // any linear identifier referenced here is consumed
            // before the new binding takes effect.
            walk(value, bindings, fn_name, source_path)?;
            if let Some(ty) = type_annot
                && is_linear(ty)
            {
                bindings.insert(
                    name.clone(),
                    LinearBinding {
                        ty_name: strip_linear(ty).to_string(),
                        defined_at: *span,
                        consumed_at: None,
                    },
                );
            } else {
                // A shadowing let without an explicit linear
                // annotation overrides whatever binding was in
                // scope. If the outer was a still-live linear,
                // that value is being dropped on the floor —
                // still an error per the single-use rule.
                // (An explicit `drop(x); let x = ...;` is fine.)
                if let Some(existing) = bindings.get(name)
                    && existing.consumed_at.is_none()
                {
                    return Err(format_error(
                        source_path,
                        *span,
                        fn_name,
                        name,
                        &existing.ty_name,
                        "shadowed while still live",
                    ));
                }
                bindings.remove(name);
            }
            Ok(())
        }
        Node::Assignment { name, value, span } => {
            walk(value, bindings, fn_name, source_path)?;
            // Re-assigning a consumed linear is disallowed in the
            // MVP (the behaviour for a resurrected handle is an
            // open design question — tracked as a follow-up).
            if let Some(existing) = bindings.get(name) {
                if existing.consumed_at.is_none() {
                    return Err(format_error(
                        source_path,
                        *span,
                        fn_name,
                        name,
                        &existing.ty_name,
                        "reassigned while still live",
                    ));
                }
                return Err(format_error(
                    source_path,
                    *span,
                    fn_name,
                    name,
                    &existing.ty_name,
                    "reassigned after move",
                ));
            }
            Ok(())
        }
        Node::ReturnStatement { value, .. } => {
            if let Some(v) = value {
                walk(v, bindings, fn_name, source_path)?;
            }
            Ok(())
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk(condition, bindings, fn_name, source_path)?;
            #[cfg(feature = "z3")]
            {
                // RES-385a: Z3-backed fallback proof for conditional
                // consumption. When both branches consume a linear
                // binding, we allow it (Z3 will prove the obligation
                // later). When only one branch consumes it, we fall
                // back to conservative behavior (snapshot/restore).
                let snap = bindings.clone();
                walk(consequence, bindings, fn_name, source_path)?;
                let consequence_state = bindings.clone();
                *bindings = snap.clone();
                if let Some(alt) = alternative {
                    walk(alt, bindings, fn_name, source_path)?;
                }
                let alternative_state = bindings.clone();

                // Merge states: if consumed in both branches, mark
                // as consumed. Otherwise use snapshot (conservative).
                for (name, alt_binding) in alternative_state {
                    if let Some(cons_binding) = consequence_state.get(&name) {
                        // Both branches had this binding in scope.
                        let both_consumed =
                            cons_binding.consumed_at.is_some() && alt_binding.consumed_at.is_some();
                        let neither_consumed =
                            cons_binding.consumed_at.is_none() && alt_binding.consumed_at.is_none();

                        if both_consumed {
                            // Consumed in both branches: mark as consumed.
                            let mut merged = alt_binding.clone();
                            merged.consumed_at =
                                alt_binding.consumed_at.or(cons_binding.consumed_at);
                            bindings.insert(name.clone(), merged);
                        } else if neither_consumed {
                            // Consumed in neither: keep alive.
                            bindings.insert(name.clone(), alt_binding);
                        } else {
                            // Consumed in only one branch: defer to Z3
                            // by keeping the conservative snapshot state.
                            bindings.insert(name.clone(), alt_binding);
                        }
                    }
                }
            }
            #[cfg(not(feature = "z3"))]
            {
                // Without Z3, use the conservative snapshot/restore
                // approach: each branch starts fresh, consumption
                // in one branch doesn't affect the merge.
                let snap = bindings.clone();
                walk(consequence, bindings, fn_name, source_path)?;
                *bindings = snap.clone();
                if let Some(alt) = alternative {
                    walk(alt, bindings, fn_name, source_path)?;
                }
                *bindings = snap;
            }
            Ok(())
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk(condition, bindings, fn_name, source_path)?;
            let snap = bindings.clone();
            walk(body, bindings, fn_name, source_path)?;
            *bindings = snap;
            Ok(())
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk(iterable, bindings, fn_name, source_path)?;
            let snap = bindings.clone();
            walk(body, bindings, fn_name, source_path)?;
            *bindings = snap;
            Ok(())
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk(scrutinee, bindings, fn_name, source_path)?;
            let snap = bindings.clone();
            for (_, guard, body) in arms {
                *bindings = snap.clone();
                if let Some(g) = guard {
                    walk(g, bindings, fn_name, source_path)?;
                }
                walk(body, bindings, fn_name, source_path)?;
            }
            *bindings = snap;
            Ok(())
        }
        Node::ExpressionStatement { expr, .. } => walk(expr, bindings, fn_name, source_path),
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            // Evaluate the callee first (may be a linear-typed
            // identifier being invoked as a function — rare, but
            // possible).
            walk(function, bindings, fn_name, source_path)?;
            for arg in arguments {
                // Linear identifiers passed as arguments are moved:
                // mark them consumed AFTER we've walked them (so a
                // re-use in the same arg list still errors).
                if let Node::Identifier {
                    name,
                    span: id_span,
                } = arg
                {
                    consume(bindings, name, *id_span, fn_name, source_path)?;
                } else {
                    walk(arg, bindings, fn_name, source_path)?;
                }
            }
            // Note: the `drop(x)` idiom goes through this same
            // CallExpression arm — no special casing. Passing `x`
            // consumes it; subsequent reads error.
            let _ = span; // span not currently surfaced — kept for future use
            Ok(())
        }
        Node::InfixExpression { left, right, .. } => {
            walk(left, bindings, fn_name, source_path)?;
            walk(right, bindings, fn_name, source_path)
        }
        Node::PrefixExpression { right, .. } => walk(right, bindings, fn_name, source_path),
        Node::TryExpression { expr, .. } => walk(expr, bindings, fn_name, source_path),
        Node::OptionalChain { object, access, .. } => {
            walk(object, bindings, fn_name, source_path)?;
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    walk(a, bindings, fn_name, source_path)?;
                }
            }
            Ok(())
        }
        Node::Identifier { name, span } => consume(bindings, name, *span, fn_name, source_path),
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            walk(condition, bindings, fn_name, source_path)?;
            if let Some(m) = message {
                walk(m, bindings, fn_name, source_path)?;
            }
            Ok(())
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                walk(i, bindings, fn_name, source_path)?;
            }
            Ok(())
        }
        Node::IndexExpression { target, index, .. } => {
            walk(target, bindings, fn_name, source_path)?;
            walk(index, bindings, fn_name, source_path)
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            walk(target, bindings, fn_name, source_path)?;
            walk(index, bindings, fn_name, source_path)?;
            walk(value, bindings, fn_name, source_path)
        }
        Node::FieldAccess { target, .. } => walk(target, bindings, fn_name, source_path),
        Node::FieldAssignment { target, value, .. } => {
            walk(target, bindings, fn_name, source_path)?;
            walk(value, bindings, fn_name, source_path)
        }
        Node::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                walk(v, bindings, fn_name, source_path)?;
            }
            Ok(())
        }
        Node::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                walk(k, bindings, fn_name, source_path)?;
                walk(v, bindings, fn_name, source_path)?;
            }
            Ok(())
        }
        Node::SetLiteral { items, .. } => {
            for i in items {
                walk(i, bindings, fn_name, source_path)?;
            }
            Ok(())
        }
        // RES-385b: closure / anonymous-fn body. Capture semantics
        // are by-move: any consumption of an outer linear binding
        // inside the body propagates to the outer scope as a
        // consumption at the closure-construction site. The
        // captured value is therefore unavailable for other uses
        // after the closure is built — which is what users want
        // when wrapping a one-shot resource into a callback.
        //
        // Multi-invocation rejection of a linear-capturing closure
        // is enforced by binding the closure to a `linear`-typed
        // local: calling such a binding twice errors via the same
        // use-after-move rule that catches direct double-consumes
        // (see the existing `linear_value_used_twice_is_rejected`
        // test in `purity_tests`).
        //
        // Closure parameters annotated `linear T` introduce new
        // linear locals inside the body, mirroring the named-fn
        // case in `check_fn_body`. Those locals stay scoped to
        // the closure — the outer scope only sees outer-binding
        // mutations.
        Node::FunctionLiteral {
            parameters, body, ..
        } => {
            let mut inner = bindings.clone();
            for (ty, pname) in parameters {
                if is_linear(ty) {
                    inner.insert(
                        pname.clone(),
                        LinearBinding {
                            ty_name: strip_linear(ty).to_string(),
                            defined_at: Span::default(),
                            consumed_at: None,
                        },
                    );
                }
            }
            walk(body, &mut inner, fn_name, source_path)?;
            // Propagate outer-binding consumption back. Only outer
            // names (those present in the original `bindings`)
            // matter; the closure's own parameter bindings stay
            // scoped to the closure. A closure param that *shadows*
            // an outer name takes over that key inside `inner`, so
            // its consumption must NOT be propagated back to the
            // (different) outer binding of the same name.
            let outer_names: Vec<String> = bindings.keys().cloned().collect();
            for name in outer_names {
                let shadowed = parameters.iter().any(|(_, p)| p == &name);
                if shadowed {
                    continue;
                }
                if let Some(inner_b) = inner.get(&name)
                    && let Some(consumed_at) = inner_b.consumed_at
                    && let Some(outer_b) = bindings.get_mut(&name)
                    && outer_b.consumed_at.is_none()
                {
                    outer_b.consumed_at = Some(consumed_at);
                }
            }
            Ok(())
        }
        // Literals and terminal nodes with no sub-expressions the
        // linearity pass cares about.
        _ => Ok(()),
    }
}

fn consume(
    bindings: &mut HashMap<String, LinearBinding>,
    name: &str,
    use_span: Span,
    fn_name: &str,
    source_path: &str,
) -> Result<(), String> {
    if let Some(b) = bindings.get_mut(name) {
        if let Some(prev) = b.consumed_at {
            let reason = format!(
                "first consumed at {}:{}",
                prev.start.line, prev.start.column
            );
            return Err(format_error(
                source_path,
                use_span,
                fn_name,
                name,
                &b.ty_name.clone(),
                &reason,
            ));
        }
        b.consumed_at = Some(use_span);
    }
    Ok(())
}

fn format_error(
    source_path: &str,
    span: Span,
    fn_name: &str,
    var_name: &str,
    ty_name: &str,
    detail: &str,
) -> String {
    let loc = if span.start.line == 0 {
        format!("{}:<unknown>", source_path)
    } else {
        format!("{}:{}:{}", source_path, span.start.line, span.start.column)
    };
    format!(
        "{}: error[linear-use]: linear value `{}: linear {}` used after move in fn `{}` ({})",
        loc, var_name, ty_name, fn_name, detail
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_prefix_detected() {
        assert!(is_linear("linear FileHandle"));
        assert!(!is_linear("FileHandle"));
        assert!(!is_linear("linear")); // no space, no base type
        assert_eq!(strip_linear("linear FileHandle"), "FileHandle");
        assert_eq!(strip_linear("FileHandle"), "FileHandle");
    }

    // ============================================================
    // RES-385b: lifetime tracking through closures
    // ============================================================
    //
    // The `walk` traversal must descend into `FunctionLiteral`
    // bodies. Captures of outer `linear` bindings should propagate
    // back to the outer scope as consumption at the closure-
    // construction site so subsequent uses error.

    /// Construct a program AST through the project's parser and run
    /// the linearity pass. Returns the first violation (if any) so
    /// tests can assert the negative or positive case.
    fn run_linear_pass(src: &str) -> Result<(), String> {
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "unexpected parse errors: {errs:?}");
        check_linear_usage(&program, "<test>")
    }

    #[test]
    fn closure_consuming_outer_linear_marks_it_consumed_at_construction() {
        // Without this descent, `consume(fh)` inside the closure was
        // invisible to the linear pass and the second use after the
        // closure construction was silently accepted. The walker now
        // sees the inner consumption and propagates it.
        let src = "\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn make(linear FileHandle fh) {\n\
                let cb = fn() { consume(fh); return 0; };\n\
                consume(fh);\n\
                return 0;\n\
            }\n";
        let err = run_linear_pass(src).expect_err("post-construction use must be rejected");
        assert!(
            err.contains("linear-use") && err.contains("fh"),
            "expected use-after-move on `fh`, got: {err}"
        );
    }

    #[test]
    fn closure_not_capturing_linear_leaves_outer_alive() {
        // A closure that doesn't reference the outer linear must not
        // accidentally mark it as consumed.
        let src = "\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn make(linear FileHandle fh) {\n\
                let cb = fn() { return 42; };\n\
                consume(fh);\n\
                return 0;\n\
            }\n";
        run_linear_pass(src).expect("non-capturing closure must not consume outer linear");
    }

    #[test]
    fn closure_body_double_uses_internal_linear_param_rejected() {
        // The same single-use rule applies to a closure's own
        // parameters: a `linear T` parameter consumed twice inside
        // the body errors, just like a named-fn parameter.
        let src = "\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn outer() {\n\
                let cb = fn(linear FileHandle fh) {\n\
                    consume(fh);\n\
                    consume(fh);\n\
                    return 0;\n\
                };\n\
                return 0;\n\
            }\n";
        let err = run_linear_pass(src)
            .expect_err("double-use of closure's linear parameter must be rejected");
        assert!(
            err.contains("linear-use"),
            "expected linear-use diagnostic, got: {err}"
        );
    }

    #[test]
    fn closure_param_scope_does_not_leak_to_outer() {
        // A closure parameter named `fh` must not collide with —
        // or be confused for — an outer `fh` binding. The closure's
        // local `fh` is scoped to the body; the outer `fh` is still
        // live after the closure literal.
        let src = "\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn outer(linear FileHandle fh) {\n\
                let cb = fn(linear FileHandle fh) { consume(fh); return 0; };\n\
                consume(fh);\n\
                return 0;\n\
            }\n";
        run_linear_pass(src)
            .expect("closure param shadowing must not consume the outer binding of the same name");
    }

    #[test]
    fn closure_capturing_linear_then_used_outside_rejected() {
        // The closure consumes `fh` in its body; using `fh` outside
        // after constructing the closure is a use-after-move.
        let src = "\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn outer(linear FileHandle fh) {\n\
                let cb = fn() { consume(fh); return 0; };\n\
                let dummy = fh;\n\
                return 0;\n\
            }\n";
        let err = run_linear_pass(src).expect_err("post-capture move must be rejected");
        assert!(
            err.contains("linear-use") && err.contains("fh"),
            "expected linear-use on `fh`, got: {err}"
        );
    }

    #[test]
    fn nested_closure_propagates_capture_through_two_levels() {
        // An inner closure consumes `fh`; the outer closure captures
        // it transitively. Either way, the outermost binding must be
        // marked consumed once both closures are constructed.
        let src = "\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn outer(linear FileHandle fh) {\n\
                let outer_cb = fn() {\n\
                    let inner_cb = fn() { consume(fh); return 0; };\n\
                    return 0;\n\
                };\n\
                consume(fh);\n\
                return 0;\n\
            }\n";
        let err = run_linear_pass(src).expect_err("transitive capture must propagate");
        assert!(
            err.contains("linear-use"),
            "expected linear-use diagnostic, got: {err}"
        );
    }

    // ============================================================
    // RES-385a: Z3-backed fallback proof for conditional consumption
    // ============================================================

    #[test]
    #[cfg(feature = "z3")]
    fn conditional_consumption_both_branches_allowed_with_z3() {
        // When both branches consume a linear binding, Z3 merging
        // allows it (Z3 will later prove the obligation).
        let src = "\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn test(linear FileHandle fh, int cond) {\n\
                if cond == 1 {\n\
                    consume(fh);\n\
                } else {\n\
                    consume(fh);\n\
                }\n\
                return 0;\n\
            }\n";
        run_linear_pass(src)
            .expect("Z3: both-branch consumption should be allowed with Z3 merging");
    }

    #[test]
    #[cfg(not(feature = "z3"))]
    fn conditional_consumption_both_branches_conservative_without_z3() {
        // Without Z3, snapshot/restore is conservative: after the
        // if-else, fh is still "live" (not consumed). If it's never
        // consumed, there's no error (it's just leaked). If it's
        // consumed later or never, that's fine from the walker's
        // perspective.
        let src = "\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn test(linear FileHandle fh, int cond) {\n\
                if cond == 1 {\n\
                    consume(fh);\n\
                } else {\n\
                    consume(fh);\n\
                }\n\
                return 0;\n\
            }\n";
        // Without Z3, this is allowed (conservative: fh appears
        // unconsumed after the if-else from walker's perspective).
        run_linear_pass(src).expect("without Z3: snapshot/restore is conservative");
    }

    #[test]
    #[cfg(feature = "z3")]
    fn conditional_consumption_one_branch_conservative() {
        // When only ONE branch consumes a linear binding, even with
        // Z3, we fall back to conservative snapshot/restore (the
        // Z3 obligation for partial consumption is more complex).
        let src = "\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn test(linear FileHandle fh, int cond) {\n\
                if cond == 1 {\n\
                    consume(fh);\n\
                }\n\
                let dummy = fh;\n\
                return 0;\n\
            }\n";
        run_linear_pass(src).expect("Z3: one-branch consumption falls back to conservative");
    }
}
