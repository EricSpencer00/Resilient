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
            // For the MVP we don't merge consumption states across
            // branches — the ticket explicitly scopes branchy paths
            // to the Z3 follow-up. We walk each branch with its own
            // snapshot so a consumption inside one arm isn't
            // observed after the branch. That makes us conservative
            // on the "used in both arms" pattern and lenient on
            // "consumed in only one arm" — the follow-up tightens
            // both.
            let snap = bindings.clone();
            walk(consequence, bindings, fn_name, source_path)?;
            *bindings = snap.clone();
            if let Some(alt) = alternative {
                walk(alt, bindings, fn_name, source_path)?;
            }
            *bindings = snap;
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
}
