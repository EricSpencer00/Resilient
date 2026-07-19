//! RES-4068 (A-E3 follow-up): `dyn Trait` trait objects — provably-sound v1.
//!
//! Scope decision recorded on the issue and in the A-E3 descope note
//! (PR #4069): Resilient's execution backends (tree-walker, VM, JIT) are
//! static/monomorphized dispatch only — there is no vtable and this pass
//! does not add one. What ships here is the **type-checking surface**:
//!
//! 1. `dyn Trait` is a real, checked type annotation (parsed by
//!    `parse_type_annotation` in `lib.rs`, encoded as the string
//!    `"dyn TraitName"`, resolved by `parse_type_name_inner` in
//!    `typechecker.rs` to `Type::Struct("dyn TraitName")`) instead of a
//!    hard parse error.
//! 2. Unknown-trait rejection: `dyn Frobnicate` where no such trait is
//!    declared, on any `dyn`-typed fn parameter, fn return type,
//!    let-binding, or struct field.
//! 3. Coercion checking: a value coerces to a `dyn Trait`-typed slot only
//!    if its concrete type provably implements `Trait` — checked wherever
//!    the concrete type is statically determinable from a struct-literal
//!    expression, mirroring the exact "struct-literal-only" philosophy
//!    `traits.rs::walk_call_sites` already uses for generic bound
//!    checking (dynamic interpreter, lightweight whole-program check, not
//!    full flow analysis). Two sites: `let x: dyn Trait = StructLiteral
//!    { .. };` and a direct call `f(StructLiteral { .. })` where `f`'s
//!    corresponding parameter is typed `dyn Trait`.
//! 4. Method-call resolution: `x.method(...)` where `x` is a fn parameter
//!    or let-binding typed `dyn Trait` is rejected when `method` is not
//!    among `Trait`'s declared methods.
//!
//! Every check above rejects *only* provable violations — a value whose
//! concrete type can't be statically determined (returned from an
//! arbitrary call, read from a field, etc.) is passed through
//! permissively, exactly as `traits.rs` already does for `<T: Trait>`
//! bound checking. Zero false positives by construction: no currently-
//! accepted program is rejected by this pass, because before this PR
//! `dyn Trait` was a hard parse error and no program using it compiled.
//!
//! RES-4095 increment 1 (PR #4143) added object-safety checking (see
//! `check_object_safety_refs` below). Increment 2 (this PR) covers
//! **dynamic dispatch execution in the tree-walker interpreter**:
//!
//! - Method calls through a `dyn Trait`-typed fn parameter or
//!   `let`-binding already reached the right concrete impl at runtime,
//!   because the tree-walker's method dispatch
//!   (`Interpreter::eval`'s `CallExpression`/`FieldAccess` arm in
//!   `lib.rs`) resolves `<expr>.method(...)` by looking up
//!   `"{runtime_struct_tag}${method}"` — the value's own concrete type,
//!   never the static `dyn Trait` annotation. No vtable was needed for
//!   this to already be correct multi-impl dynamic dispatch.
//! - What *didn't* work: reassigning a `dyn`-typed variable to a
//!   *different* concrete type mid-function. `Node::Assignment`'s type
//!   check compared the new value's type directly against the
//!   variable's declared type and rejected any mismatch, so `c = new
//!   Square {..}` after `let c: dyn Shape = new Circle {..}` was a hard
//!   type error — defeating the entire point of `dyn Trait` (holding
//!   heterogeneous impls in one binding over time). Fixed by extending
//!   `typechecker.rs`'s `Node::Assignment` arm with the same
//!   `dyn `-prefix permissive gate `type_satisfies` already applies to
//!   the initial `let` binding, and extending this module's
//!   `check_method_calls` (renamed in spirit, not in name — it now also
//!   covers reassignment) to reject a reassignment whose RHS is a
//!   direct struct literal that provably doesn't implement the trait,
//!   mirroring the `let`/call-site coercion checks below.
//!
//! Increment 3 (RES-4095) fixed the VM/JIT reassignment divergence noted
//! above: the bytecode VM's `Op::CallMethod` already dispatches by the
//! receiver's *runtime* struct tag (matching the tree-walker), so no
//! vtable was actually needed. The real bug was in the unrelated
//! compile-time `devirtualize.rs` optimization pass, which rewrites a
//! statically-known `x.method()` into a direct call and never
//! invalidated its binding on `Node::Assignment` — see that file's
//! `Node::Assignment` arm in `rewrite_node`.
//!
//! Deferred to a follow-up issue: `dyn Trait` in generic/container
//! position, and flow-sensitive coercion checking beyond the direct
//! literal call/let/assignment sites above (e.g. a `dyn`-typed variable
//! reassigned *through another variable*
//! before reaching a call site).

use crate::Node;
use crate::span::Span;
use crate::uniqueness_walk;
use std::collections::{HashMap, HashSet};

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };

    // Pass 1: collect trait declarations (name -> declared method names).
    // RES-1802-style pre-size: at most one entry per top-level TraitDecl.
    let mut trait_methods: HashMap<String, HashSet<String>> = HashMap::with_capacity(4);
    // RES-4095: name -> declared method sigs, kept alongside the name-set
    // above so `check_object_safety_refs` can inspect `takes_self` /
    // `returns_self` without re-walking the program.
    let mut trait_sigs: HashMap<String, &Vec<crate::traits::TraitMethodSig>> =
        HashMap::with_capacity(4);
    for stmt in stmts {
        if let Node::TraitDecl { name, methods, .. } = &stmt.node {
            trait_methods.insert(
                name.clone(),
                methods.iter().map(|m| m.name.clone()).collect(),
            );
            trait_sigs.insert(name.clone(), methods);
        }
    }

    // Pass 2: collect struct method coverage (struct -> method -> arity)
    // and explicit `impl Trait for Type` pairs, exactly like
    // `traits.rs::check`'s Pass 1B — duplicated here (rather than shared)
    // to keep this feature's logic self-contained per the feature-
    // isolation convention.
    let mut type_methods: HashMap<String, HashMap<String, usize>> =
        HashMap::with_capacity(stmts.len());
    let mut explicit_impls: HashSet<(String, String)> = HashSet::with_capacity(stmts.len());
    for stmt in stmts {
        if let Node::ImplBlock {
            trait_name,
            struct_name,
            methods,
            ..
        } = &stmt.node
        {
            let entry = type_methods.entry(struct_name.clone()).or_default();
            for m in methods {
                if let Node::Function {
                    name, parameters, ..
                } = m
                {
                    let plain = name
                        .strip_prefix(&format!("{}$", struct_name))
                        .unwrap_or(name);
                    entry.insert(plain.to_string(), parameters.len());
                }
            }
            if let Some(t) = trait_name {
                explicit_impls.insert((t.clone(), struct_name.clone()));
            }
        }
    }

    let satisfies = |trait_name: &str, struct_name: &str| -> bool {
        if explicit_impls.contains(&(trait_name.to_string(), struct_name.to_string())) {
            return true;
        }
        match (trait_methods.get(trait_name), type_methods.get(struct_name)) {
            (Some(tm), Some(sm)) => tm.iter().all(|m| sm.contains_key(m)),
            _ => false,
        }
    };

    // Pass 3: unknown-trait rejection on every `dyn X` annotation —
    // fn parameters, fn return types, let-bindings, struct fields.
    for stmt in stmts {
        check_unknown_trait_refs(&stmt.node, &trait_methods, source_path)?;
    }

    // RES-4095: Pass 3.5 — object-safety rejection on every `dyn X`
    // annotation where `X` is a *known* trait (unknown traits are
    // already rejected by Pass 3 above, so this pass only needs to
    // reason about traits that resolved). Runs before coercion/method-
    // call checking so an object-safety violation is reported instead
    // of a confusing downstream coercion or method-resolution error.
    for stmt in stmts {
        check_object_safety_refs(&stmt.node, &trait_sigs, source_path)?;
    }

    // Pass 4: coercion checking at struct-literal let-bindings and
    // direct call sites, plus method-call resolution against each
    // top-level fn's `dyn`-typed parameters and let-bindings.
    let fns_by_name: HashMap<&str, &Node> = stmts
        .iter()
        .filter_map(|s| {
            if let Node::Function { name, .. } = &s.node {
                Some((name.as_str(), &s.node as &Node))
            } else {
                None
            }
        })
        .collect();

    // RES-4095 increment 4: struct-field type map (struct -> field ->
    // declared type), needed so `check_coercions` can verify a `dyn
    // Trait`- or `Array<dyn Trait>`-typed field assigned in a struct
    // literal (issue item 5).
    let mut struct_field_types: HashMap<String, HashMap<String, String>> =
        HashMap::with_capacity(stmts.len());
    for stmt in stmts {
        if let Node::StructDecl { name, fields, .. } = &stmt.node {
            let entry = struct_field_types.entry(name.clone()).or_default();
            for (ftype, fname) in fields {
                entry.insert(fname.clone(), ftype.clone());
            }
        }
    }

    for stmt in stmts {
        check_coercions(
            &stmt.node,
            &fns_by_name,
            &struct_field_types,
            &satisfies,
            source_path,
        )?;
        if let Node::Function {
            parameters, body, ..
        } = &stmt.node
        {
            check_method_calls(parameters, body, &trait_methods, &satisfies, source_path)?;
        }
    }

    Ok(())
}

/// Returns the trait name if `annot` is a `dyn TraitName` annotation
/// (stripping the `linear ` prefix RES-385 may have added first).
fn dyn_trait_name(annot: &str) -> Option<&str> {
    crate::linear::strip_linear(annot).strip_prefix("dyn ")
}

/// RES-4095 increment 4: like `dyn_trait_name`, but also matches `dyn
/// Trait` nested as the first type parameter of a container annotation,
/// e.g. `Array<dyn Shape>` or `Option<dyn Shape>` (`Name<T1, T2, ...>`
/// is encoded verbatim by `parse_type_annotation` in lib.rs, so this is
/// a plain string match on the first comma-separated parameter).
///
/// Used only for unknown-trait / object-safety rejection — safe to
/// apply at any nesting depth, since rejecting an unknown or object-
/// not-object-safe trait can never turn a previously-rejected program into an
/// accepted one. Deliberately NOT used for method-call/`dyn_vars`
/// tracking or reassignment checks: a container-typed binding is not
/// itself a `dyn Trait` value, and treating it as one there would
/// falsely reject legitimate container methods (`.len()`, `.push()`,
/// …) — see `check_method_calls`'s doc comment.
fn dyn_trait_name_any_depth(annot: &str) -> Option<&str> {
    let stripped = crate::linear::strip_linear(annot);
    if let Some(t) = stripped.strip_prefix("dyn ") {
        return Some(t);
    }
    let inner = stripped.strip_suffix('>')?;
    let lt = inner.find('<')?;
    let first_param = inner[lt + 1..].split(',').next()?.trim();
    first_param.strip_prefix("dyn ")
}

/// RES-4095 increment 4: matches `Array<dyn Trait>` specifically (not
/// `Option<dyn Trait>` or other containers) — used by `check_coercions`
/// to verify each element of an `Array<dyn Trait>`-typed slot's
/// `ArrayLiteral` value, mirroring the existing struct-literal coercion
/// checks below at the same "direct literal expression" precision.
fn array_dyn_trait_name(annot: &str) -> Option<&str> {
    let stripped = crate::linear::strip_linear(annot);
    stripped
        .strip_prefix("Array<")?
        .strip_suffix('>')?
        .strip_prefix("dyn ")
}

fn check_unknown_trait_refs(
    node: &Node,
    trait_methods: &HashMap<String, HashSet<String>>,
    source_path: &str,
) -> Result<(), String> {
    let mut err: Option<String> = None;
    uniqueness_walk::visit(node, &mut |n| {
        if err.is_some() {
            return;
        }
        match n {
            Node::Function {
                parameters,
                return_type,
                span,
                ..
            } => {
                for (ptype, _) in parameters {
                    if let Some(t) = dyn_trait_name_any_depth(ptype)
                        && !trait_methods.contains_key(t)
                    {
                        err = Some(format_err(
                            source_path,
                            *span,
                            &format!("unknown trait `{}` in `dyn {}`", t, t),
                        ));
                        return;
                    }
                }
                if let Some(rt) = return_type
                    && let Some(t) = dyn_trait_name_any_depth(rt)
                    && !trait_methods.contains_key(t)
                {
                    err = Some(format_err(
                        source_path,
                        *span,
                        &format!("unknown trait `{}` in `dyn {}`", t, t),
                    ));
                }
            }
            Node::LetStatement {
                type_annot, span, ..
            } => {
                if let Some(ann) = type_annot
                    && let Some(t) = dyn_trait_name_any_depth(ann)
                    && !trait_methods.contains_key(t)
                {
                    err = Some(format_err(
                        source_path,
                        *span,
                        &format!("unknown trait `{}` in `dyn {}`", t, t),
                    ));
                }
            }
            Node::StructDecl { fields, span, .. } => {
                for (ftype, _) in fields {
                    if let Some(t) = dyn_trait_name_any_depth(ftype)
                        && !trait_methods.contains_key(t)
                    {
                        err = Some(format_err(
                            source_path,
                            *span,
                            &format!("unknown trait `{}` in `dyn {}`", t, t),
                        ));
                        return;
                    }
                }
            }
            _ => {}
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// RES-4095: object-safety rules for `dyn Trait`. Each rule below rejects
/// only a *statically provable* shape — no vtable exists in any backend
/// today (see this module's doc comment), so this is a pure static-
/// analysis gate that runs ahead of the (still-deferred) codegen work in
/// this ticket's follow-up PRs.
///
/// - A method with no `self` receiver (`takes_self == false`) has
///   nothing for a `dyn Trait` call to dispatch through — there is no
///   erased receiver value to route the call to.
/// - A method that returns `Self` promises to hand back the callee's
///   own concrete type, which a `dyn Trait` caller — who by definition
///   doesn't know the concrete type — cannot be given.
///
/// Returns `Some((method_name, reason))` for the first violation found
/// (declaration order), or `None` if every method is dispatchable.
fn object_safety_violation(sigs: &[crate::traits::TraitMethodSig]) -> Option<(&str, &'static str)> {
    for m in sigs {
        if !m.takes_self {
            return Some((
                m.name.as_str(),
                "has no `self` receiver, so it cannot be dispatched through a `dyn Trait` value",
            ));
        }
        if m.returns_self {
            return Some((
                m.name.as_str(),
                "returns `Self`, which a `dyn Trait` caller cannot be given \
                 (the concrete type is erased)",
            ));
        }
    }
    None
}

fn check_object_safety_refs(
    node: &Node,
    trait_sigs: &HashMap<String, &Vec<crate::traits::TraitMethodSig>>,
    source_path: &str,
) -> Result<(), String> {
    let mut err: Option<String> = None;
    let report = |trait_name: &str, span: Span| -> Option<String> {
        let sigs = trait_sigs.get(trait_name)?;
        let (method, reason) = object_safety_violation(sigs)?;
        Some(format_err(
            source_path,
            span,
            &format!(
                "[E0021] `dyn {}` is not object-safe: method `{}` {}",
                trait_name, method, reason
            ),
        ))
    };
    uniqueness_walk::visit(node, &mut |n| {
        if err.is_some() {
            return;
        }
        match n {
            Node::Function {
                parameters,
                return_type,
                span,
                ..
            } => {
                for (ptype, _) in parameters {
                    if let Some(t) = dyn_trait_name_any_depth(ptype)
                        && let Some(e) = report(t, *span)
                    {
                        err = Some(e);
                        return;
                    }
                }
                if let Some(rt) = return_type
                    && let Some(t) = dyn_trait_name_any_depth(rt)
                    && let Some(e) = report(t, *span)
                {
                    err = Some(e);
                }
            }
            Node::LetStatement {
                type_annot, span, ..
            } => {
                if let Some(ann) = type_annot
                    && let Some(t) = dyn_trait_name_any_depth(ann)
                    && let Some(e) = report(t, *span)
                {
                    err = Some(e);
                }
            }
            Node::StructDecl { fields, span, .. } => {
                for (ftype, _) in fields {
                    if let Some(t) = dyn_trait_name_any_depth(ftype)
                        && let Some(e) = report(t, *span)
                    {
                        err = Some(e);
                        return;
                    }
                }
            }
            _ => {}
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Checks a single candidate coercion site: `value` flowing into a slot
/// declared `dyn Trait` or `Array<dyn Trait>`. Only direct literal
/// expressions are checked (a bare `StructLiteral`, or an `ArrayLiteral`
/// whose elements are themselves direct `StructLiteral`s) — anything
/// else (a call result, a variable read, …) is not statically
/// determinable and is passed through permissively, matching this
/// module's existing precision everywhere else.
fn check_one_coercion(
    declared_type: &str,
    value: &Node,
    satisfies: &impl Fn(&str, &str) -> bool,
    describe: impl Fn(&str, &str) -> String,
) -> Option<String> {
    if let Some(trait_name) = dyn_trait_name(declared_type) {
        if let Node::StructLiteral { name, .. } = value
            && !satisfies(trait_name, name)
        {
            return Some(describe(trait_name, name));
        }
        return None;
    }
    if let Some(trait_name) = array_dyn_trait_name(declared_type)
        && let Node::ArrayLiteral { items, .. } = value
    {
        for item in items {
            if let Node::StructLiteral { name, .. } = item
                && !satisfies(trait_name, name)
            {
                return Some(describe(trait_name, name));
            }
        }
    }
    None
}

fn check_coercions(
    node: &Node,
    fns_by_name: &HashMap<&str, &Node>,
    struct_field_types: &HashMap<String, HashMap<String, String>>,
    satisfies: &impl Fn(&str, &str) -> bool,
    source_path: &str,
) -> Result<(), String> {
    let mut err: Option<String> = None;
    uniqueness_walk::visit(node, &mut |n| {
        if err.is_some() {
            return;
        }
        match n {
            Node::LetStatement {
                type_annot: Some(ann),
                value,
                span,
                ..
            } => {
                if let Some(msg) = check_one_coercion(ann, value, satisfies, |trait_name, name| {
                    format!(
                        "type `{}` does not implement `{}`, required to coerce to `dyn {}`",
                        name, trait_name, trait_name
                    )
                }) {
                    err = Some(format_err(source_path, *span, &msg));
                }
            }
            Node::CallExpression {
                function,
                arguments,
                span,
            } => {
                let callee_name = match function.as_ref() {
                    Node::Identifier { name, .. } => name.as_str(),
                    _ => return,
                };
                let Some(Node::Function { parameters, .. }) = fns_by_name.get(callee_name).copied()
                else {
                    return;
                };
                for (i, (ptype, _)) in parameters.iter().enumerate() {
                    let Some(arg) = arguments.get(i) else {
                        continue;
                    };
                    if let Some(msg) = check_one_coercion(
                        ptype,
                        arg,
                        satisfies,
                        |trait_name, name| {
                            format!(
                                "type `{}` does not implement `{}`, required to coerce to `dyn {}` (argument {} of `{}`)",
                                name,
                                trait_name,
                                trait_name,
                                i + 1,
                                callee_name
                            )
                        },
                    ) {
                        err = Some(format_err(source_path, *span, &msg));
                        return;
                    }
                }
            }
            // RES-4095 increment 4 (issue item 5): a struct field typed
            // `dyn Trait` / `Array<dyn Trait>`, assigned in a struct
            // literal.
            Node::StructLiteral {
                name: struct_name,
                fields,
                span,
                ..
            } => {
                let Some(field_types) = struct_field_types.get(struct_name) else {
                    return;
                };
                for (fname, fvalue) in fields {
                    let Some(ftype) = field_types.get(fname) else {
                        continue;
                    };
                    if let Some(msg) = check_one_coercion(
                        ftype,
                        fvalue,
                        satisfies,
                        |trait_name, name| {
                            format!(
                                "type `{}` does not implement `{}`, required to coerce to `dyn {}` (field `{}` of `{}`)",
                                name, trait_name, trait_name, fname, struct_name
                            )
                        },
                    ) {
                        err = Some(format_err(source_path, *span, &msg));
                        return;
                    }
                }
            }
            // RES-4095 increment 4 (issue item 5): a fn returning `dyn
            // Trait` / `Array<dyn Trait>`, checked against direct
            // literal `return` expressions in its body.
            Node::Function {
                return_type: Some(rt),
                body,
                name: fn_name,
                span,
                ..
            } => {
                uniqueness_walk::visit(body, &mut |bn| {
                    if err.is_some() {
                        return;
                    }
                    if let Node::ReturnStatement {
                        value: Some(rv),
                        span: rspan,
                    } = bn
                        && let Some(msg) = check_one_coercion(
                            rt,
                            rv,
                            satisfies,
                            |trait_name, name| {
                                format!(
                                    "type `{}` does not implement `{}`, required to coerce to `dyn {}` (return value of `{}`)",
                                    name, trait_name, trait_name, fn_name
                                )
                            },
                        )
                    {
                        err = Some(format_err(
                            source_path,
                            if rspan.start.line == 0 { *span } else { *rspan },
                            &msg,
                        ));
                    }
                });
            }
            _ => {}
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Method-call resolution against `dyn Trait`-typed parameters and
/// let-bindings inside a single function, plus reassignment-coercion
/// checking (RES-4095) on the same `dyn`-typed bindings. Deliberately
/// flat and non-scoped (every `dyn`-typed binding in the whole function
/// body is collected into one map, regardless of nested-block
/// shadowing) — under-checking a shadowed rebind is acceptable
/// (permissive), never over-rejecting.
fn check_method_calls(
    parameters: &[(String, String)],
    body: &Node,
    trait_methods: &HashMap<String, HashSet<String>>,
    satisfies: &impl Fn(&str, &str) -> bool,
    source_path: &str,
) -> Result<(), String> {
    let mut dyn_vars: HashMap<String, String> = HashMap::new();
    for (ptype, pname) in parameters {
        if let Some(t) = dyn_trait_name(ptype) {
            dyn_vars.insert(pname.clone(), t.to_string());
        }
    }
    uniqueness_walk::visit(body, &mut |n| {
        if let Node::LetStatement {
            name,
            type_annot: Some(ann),
            ..
        } = n
            && let Some(t) = dyn_trait_name(ann)
        {
            dyn_vars.insert(name.clone(), t.to_string());
        }
    });

    if dyn_vars.is_empty() {
        return Ok(());
    }

    let mut err: Option<String> = None;
    uniqueness_walk::visit(body, &mut |n| {
        if err.is_some() {
            return;
        }
        if let Node::CallExpression { function, span, .. } = n
            && let Node::FieldAccess { target, field, .. } = function.as_ref()
            && let Node::Identifier { name: var, .. } = target.as_ref()
            && let Some(trait_name) = dyn_vars.get(var)
        {
            let known = trait_methods
                .get(trait_name)
                .is_some_and(|ms| ms.contains(field));
            if !known {
                err = Some(format_err(
                    source_path,
                    *span,
                    &format!(
                        "no method `{}` on `dyn {}` (`{}` declares no such method)",
                        field, trait_name, trait_name
                    ),
                ));
            }
            return;
        }
        // RES-4095: reassigning a `dyn Trait`-typed binding to a new
        // concrete value is the mechanism that makes runtime dispatch
        // actually dynamic — reject it only when the RHS is a direct
        // struct literal (statically determinable) that provably does
        // not implement the trait, mirroring `check_coercions`'s
        // let-binding and call-site checks above.
        if let Node::Assignment {
            name: var,
            value,
            span,
        } = n
            && let Some(trait_name) = dyn_vars.get(var)
            && let Node::StructLiteral {
                name: struct_name, ..
            } = value.as_ref()
            && !satisfies(trait_name, struct_name)
        {
            err = Some(format_err(
                source_path,
                *span,
                &format!(
                    "type `{}` does not implement `{}`, required to coerce to `dyn {}`",
                    struct_name, trait_name, trait_name
                ),
            ));
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn format_err(source_path: &str, span: Span, msg: &str) -> String {
    if span.start.line == 0 {
        msg.to_string()
    } else {
        format!(
            "{}:{}:{}: {}",
            source_path, span.start.line, span.start.column, msg
        )
    }
}

#[cfg(test)]
mod tests {
    fn check_source(src: &str) -> Result<(), String> {
        let (program, errors) = crate::parse(src);
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        super::check(&program, "test.rz")
    }

    #[test]
    fn accepts_dyn_trait_type_annotation() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Circle { int r, }
            impl Shape for Circle {
                fn area(self) -> int { return self.r; }
            }
            fn use_shape(dyn Shape s) -> int {
                return s.area();
            }
            fn main() {
                use_shape(new Circle { r: 3 });
            }
            main();
        "#;
        assert!(check_source(src).is_ok(), "{:?}", check_source(src));
    }

    #[test]
    fn rejects_unknown_trait_in_dyn() {
        let src = r#"
            fn use_shape(dyn Frobnicate s) -> int {
                return 0;
            }
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("unknown trait `Frobnicate`"), "{}", err);
    }

    #[test]
    fn rejects_coercion_when_struct_does_not_implement_trait() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Square { int side, }
            fn use_shape(dyn Shape s) -> int {
                return 0;
            }
            fn main() {
                use_shape(new Square { side: 2 });
            }
            main();
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("does not implement `Shape`"), "{}", err);
    }

    #[test]
    fn rejects_let_coercion_when_struct_does_not_implement_trait() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Square { int side, }
            fn main() {
                let s: dyn Shape = new Square { side: 2 };
            }
            main();
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("does not implement `Shape`"), "{}", err);
    }

    #[test]
    fn accepts_let_coercion_when_struct_implements_trait_structurally() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Circle { int r, }
            impl Circle {
                fn area(self) -> int { return self.r; }
            }
            fn main() {
                let s: dyn Shape = new Circle { r: 3 };
            }
            main();
        "#;
        assert!(check_source(src).is_ok(), "{:?}", check_source(src));
    }

    #[test]
    fn rejects_unknown_method_on_dyn_trait() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Circle { int r, }
            impl Shape for Circle {
                fn area(self) -> int { return self.r; }
            }
            fn use_shape(dyn Shape s) -> int {
                return s.perimeter();
            }
        "#;
        let err = check_source(src).unwrap_err();
        assert!(
            err.contains("no method `perimeter` on `dyn Shape`"),
            "{}",
            err
        );
    }

    #[test]
    fn permissive_on_non_literal_coercion() {
        // The concrete type isn't statically determinable here (it's a
        // return value from an arbitrary fn) — must NOT be rejected.
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Square { int side, }
            fn make() -> Square {
                return new Square { side: 2 };
            }
            fn use_shape(dyn Shape s) -> int {
                return 0;
            }
            fn main() {
                use_shape(make());
            }
            main();
        "#;
        assert!(check_source(src).is_ok(), "{:?}", check_source(src));
    }

    // RES-4095: object-safety checking.

    #[test]
    fn rejects_dyn_trait_with_no_self_method() {
        let src = r#"
            trait Factory {
                fn make() -> int;
            }
            fn use_factory(dyn Factory f) -> int {
                return 0;
            }
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("[E0021]"), "{}", err);
        assert!(err.contains("not object-safe"), "{}", err);
        assert!(err.contains("no `self` receiver"), "{}", err);
    }

    #[test]
    fn rejects_dyn_trait_with_self_returning_method() {
        let src = r#"
            trait Cloneable {
                fn duplicate(self) -> Self;
            }
            fn use_cloneable(dyn Cloneable c) -> int {
                return 0;
            }
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("[E0021]"), "{}", err);
        assert!(err.contains("returns `Self`"), "{}", err);
    }

    #[test]
    fn rejects_object_unsafe_trait_in_return_type() {
        let src = r#"
            trait Cloneable {
                fn duplicate(self) -> Self;
            }
            fn get_cloneable() -> dyn Cloneable {
                return 0;
            }
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("[E0021]"), "{}", err);
    }

    #[test]
    fn rejects_object_unsafe_trait_in_let_binding() {
        let src = r#"
            trait Cloneable {
                fn duplicate(self) -> Self;
            }
            struct Widget { int id, }
            impl Cloneable for Widget {
                fn duplicate(self) -> Widget { return self; }
            }
            fn main() {
                let w: dyn Cloneable = new Widget { id: 1 };
            }
            main();
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("[E0021]"), "{}", err);
    }

    #[test]
    fn rejects_object_unsafe_trait_in_struct_field() {
        let src = r#"
            trait Cloneable {
                fn duplicate(self) -> Self;
            }
            struct Holder {
                dyn Cloneable inner,
            }
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("[E0021]"), "{}", err);
    }

    #[test]
    fn accepts_object_safe_trait_with_self_typed_impl_return() {
        // The trait method returns the *impl*'s concrete type (`Circle`),
        // not the literal identifier `Self` — that's a perfectly
        // dispatchable, object-safe method (unrelated to E0021, which
        // only fires on the bare `Self` return-type annotation on the
        // trait declaration itself).
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Circle { int r, }
            impl Shape for Circle {
                fn area(self) -> int { return self.r; }
            }
            fn use_shape(dyn Shape s) -> int {
                return s.area();
            }
            fn main() {
                use_shape(new Circle { r: 3 });
            }
            main();
        "#;
        assert!(check_source(src).is_ok(), "{:?}", check_source(src));
    }

    // RES-4095 increment 2: reassignment through a `dyn Trait` binding.

    #[test]
    fn accepts_reassigning_dyn_binding_to_different_implementing_struct() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Circle { int r, }
            struct Square { int s, }
            impl Shape for Circle {
                fn area(self) -> int { return self.r; }
            }
            impl Shape for Square {
                fn area(self) -> int { return self.s; }
            }
            fn main() {
                let c: dyn Shape = new Circle { r: 3 };
                c = new Square { s: 4 };
            }
            main();
        "#;
        assert!(check_source(src).is_ok(), "{:?}", check_source(src));
    }

    #[test]
    fn rejects_reassigning_dyn_binding_to_non_implementing_struct() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Circle { int r, }
            struct NotAShape { int x, }
            impl Shape for Circle {
                fn area(self) -> int { return self.r; }
            }
            fn main() {
                let c: dyn Shape = new Circle { r: 3 };
                c = new NotAShape { x: 1 };
            }
            main();
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("does not implement `Shape`"), "{}", err);
    }

    #[test]
    fn permissive_on_non_literal_reassignment() {
        // Mirrors `permissive_on_non_literal_coercion` above: the RHS
        // isn't a direct struct literal (it's a call result), so its
        // concrete type isn't statically determinable — must not be
        // rejected.
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Circle { int r, }
            impl Shape for Circle {
                fn area(self) -> int { return self.r; }
            }
            fn make() -> Circle {
                return new Circle { r: 5 };
            }
            fn main() {
                let c: dyn Shape = new Circle { r: 3 };
                c = make();
            }
            main();
        "#;
        assert!(check_source(src).is_ok(), "{:?}", check_source(src));
    }

    // RES-4095 increment 4: `dyn Trait` in generic/container position.

    #[test]
    fn rejects_unknown_trait_in_array_dyn() {
        let src = r#"
            fn use_shapes(Array<dyn Frobnicate> shapes) -> int {
                return 0;
            }
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("unknown trait `Frobnicate`"), "{}", err);
    }

    #[test]
    fn rejects_not_object_safe_trait_in_array_dyn() {
        let src = r#"
            trait Factory {
                fn make() -> int;
            }
            fn use_factories(Array<dyn Factory> factories) -> int {
                return 0;
            }
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("[E0021]"), "{}", err);
    }

    #[test]
    fn rejects_not_object_safe_trait_in_array_dyn_let_binding() {
        let src = r#"
            trait Factory {
                fn make() -> int;
            }
            fn main() {
                let fs: Array<dyn Factory> = [];
            }
            main();
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("[E0021]"), "{}", err);
    }

    #[test]
    fn rejects_array_dyn_coercion_when_element_does_not_implement_trait() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Circle { int r, }
            struct Square { int side, }
            impl Shape for Circle {
                fn area(self) -> int { return self.r; }
            }
            fn main() {
                let shapes: Array<dyn Shape> = [new Circle { r: 2 }, new Square { side: 3 }];
            }
            main();
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("does not implement `Shape`"), "{}", err);
    }

    #[test]
    fn accepts_array_dyn_coercion_when_every_element_implements_trait() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Circle { int r, }
            struct Square { int s, }
            impl Shape for Circle {
                fn area(self) -> int { return self.r; }
            }
            impl Shape for Square {
                fn area(self) -> int { return self.s; }
            }
            fn main() {
                let shapes: Array<dyn Shape> = [new Circle { r: 2 }, new Square { s: 3 }];
            }
            main();
        "#;
        assert!(check_source(src).is_ok(), "{:?}", check_source(src));
    }

    #[test]
    fn rejects_array_dyn_coercion_at_call_site() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Square { int side, }
            fn use_shapes(Array<dyn Shape> shapes) -> int {
                return 0;
            }
            fn main() {
                use_shapes([new Square { side: 2 }]);
            }
            main();
        "#;
        let err = check_source(src).unwrap_err();
        assert!(err.contains("does not implement `Shape`"), "{}", err);
    }

    #[test]
    fn rejects_struct_field_coercion_when_value_does_not_implement_trait() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Square { int side, }
            struct Holder {
                dyn Shape inner,
            }
            fn main() {
                let h = new Holder { inner: new Square { side: 2 } };
            }
            main();
        "#;
        let err = check_source(src).unwrap_err();
        assert!(
            err.contains("does not implement `Shape`") && err.contains("field `inner`"),
            "{}",
            err
        );
    }

    #[test]
    fn accepts_struct_field_coercion_when_value_implements_trait() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Circle { int r, }
            impl Shape for Circle {
                fn area(self) -> int { return self.r; }
            }
            struct Holder {
                dyn Shape inner,
            }
            fn main() {
                let h = new Holder { inner: new Circle { r: 2 } };
            }
            main();
        "#;
        assert!(check_source(src).is_ok(), "{:?}", check_source(src));
    }

    #[test]
    fn rejects_return_type_coercion_when_value_does_not_implement_trait() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Square { int side, }
            fn get_shape() -> dyn Shape {
                return new Square { side: 2 };
            }
        "#;
        let err = check_source(src).unwrap_err();
        assert!(
            err.contains("does not implement `Shape`") && err.contains("return value"),
            "{}",
            err
        );
    }

    #[test]
    fn accepts_return_type_coercion_when_value_implements_trait() {
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Circle { int r, }
            impl Shape for Circle {
                fn area(self) -> int { return self.r; }
            }
            fn get_shape() -> dyn Shape {
                return new Circle { r: 2 };
            }
        "#;
        assert!(check_source(src).is_ok(), "{:?}", check_source(src));
    }

    #[test]
    fn permissive_on_non_literal_array_element() {
        // Mirrors `permissive_on_non_literal_coercion`: an element that
        // isn't a direct struct literal (a call result) isn't
        // statically determinable — must not be rejected.
        let src = r#"
            trait Shape {
                fn area(self) -> int;
            }
            struct Square { int side, }
            fn make() -> Square {
                return new Square { side: 2 };
            }
            fn main() {
                let shapes: Array<dyn Shape> = [make()];
            }
            main();
        "#;
        assert!(check_source(src).is_ok(), "{:?}", check_source(src));
    }
}
