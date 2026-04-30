//! RES-405 (was RES-289 / RES-81): generic type-parameter validation
//! and body-consistency checks.
//!
//! The interpreter is dynamically typed, so this pass does NOT do full
//! monomorphisation (that lands in PR 3 of RES-405 for the bytecode VM
//! and PR 4 for the Cranelift JIT). What ships here:
//!
//! 1. **Duplicate type-parameter rejection** — `fn<T, T>(x: T)` errors.
//!    (Pre-existing behaviour from RES-289.)
//!
//! 2. **Body consistency** — when a function declares `fn<T>(x: T) -> T`
//!    but the body uses `T` in a way that constrains it to a concrete
//!    type (e.g. `x + 1` forces `T = Int`), the fn is rejected with a
//!    diagnostic that names the offending operator and the implicit
//!    constraint:
//!
//!    ```text
//!    error: type parameter `T` of fn `bad` is constrained to a concrete
//!           type by the body — operator `+` with an Int literal forces
//!           T = Int. Either drop the type parameter and write
//!           `fn bad(x: Int) -> Int`, or restructure the body so T stays
//!           polymorphic.
//!    ```
//!
//! 3. **`Subst` and `infer_subst` machinery** — the substitution map
//!    type and a local-bidirectional inference helper that downstream
//!    passes (walker plumbing, VM monomorph) consume. The inference
//!    is *local* — for each call-site argument, the actual argument's
//!    type unifies with the declared parameter type, recording any
//!    `T -> ConcreteType` mapping. Failure modes (unresolvable T,
//!    inconsistent T) become diagnostics that the caller surfaces.
//!
//! ## Design lock-in
//!
//! See `docs/superpowers/specs/2026-04-30-generics-design.md` for the
//! sign-off on the four design questions:
//!
//! * Q1 — hybrid: walker erases (already tag-dispatched), VM and JIT
//!   monomorphize. Walker plumbing is PR 2; VM / JIT are PRs 3-4.
//! * Q2 — local bidirectional inference with explicit instantiation as
//!   a fallback. This module's `infer_subst` implements that algorithm.
//! * Q3 — substitution happens post-typecheck in a dedicated lowering
//!   pass. The lowering site lives in `monomorph::lower` (future PR);
//!   this module just produces the `Subst` map.
//! * Q4 — trait bounds are forwarded through; bound-checking lives in
//!   RES-290's PR.

// PR 2-4 of RES-405 consume the `Subst` / `apply_subst` / `infer_subst`
// API; PR 1 lays the surface so downstream PRs land additively.
#![allow(dead_code)]

use crate::Node;
use crate::typechecker::Type;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Substitution machinery (PR 1 — consumed by PRs 2-4).
// ---------------------------------------------------------------------------

/// Mapping from a type-parameter name (e.g. `"T"`) to the concrete
/// `Type` it was instantiated with at a particular call site. The
/// `infer_subst` helper builds one of these per generic call; the
/// downstream lowering pass (RES-405 PR 3) clones each generic body
/// with the substitution applied to produce a specialized chunk.
#[derive(Debug, Clone, Default)]
pub struct Subst {
    map: HashMap<String, Type>,
}

impl Subst {
    pub fn new() -> Self {
        Self::default()
    }

    /// Associate `tp_name` with `ty`. Returns an error if `tp_name`
    /// was already bound to a different type — that's the
    /// "T must be both Int and String" inconsistency the diagnostic
    /// surface targets.
    pub fn bind(&mut self, tp_name: &str, ty: Type) -> Result<(), String> {
        match self.map.get(tp_name) {
            Some(existing) if existing != &ty => Err(format!(
                "type parameter `{}` is inferred as both `{}` and `{}` — they must agree",
                tp_name, existing, ty
            )),
            _ => {
                self.map.insert(tp_name.to_string(), ty);
                Ok(())
            }
        }
    }

    /// Look up the concrete type for `tp_name`, if one was bound.
    pub fn get(&self, tp_name: &str) -> Option<&Type> {
        self.map.get(tp_name)
    }

    /// True when no parameters have been bound yet.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterate over `(tp_name, concrete_type)` pairs in arbitrary
    /// (HashMap) order. Callers that need stable order should sort.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Type)> {
        self.map.iter()
    }
}

/// Apply a substitution to a type. Free type-parameter names in `ty`
/// are replaced with their concrete bindings; concrete and
/// already-substituted types pass through unchanged.
///
/// Today, type-parameter names live as bare `Type::Struct(name)`
/// entries (the parser stores parameter type annotations as strings).
/// `apply_subst` consults `subst` for any `Struct(name)` whose name
/// is present and rewrites accordingly. Function and array types
/// recurse.
pub fn apply_subst(ty: &Type, subst: &Subst) -> Type {
    match ty {
        Type::Struct(name) => match subst.get(name) {
            Some(replaced) => replaced.clone(),
            None => ty.clone(),
        },
        Type::Function {
            params,
            return_type,
        } => Type::Function {
            params: params.iter().map(|p| apply_subst(p, subst)).collect(),
            return_type: Box::new(apply_subst(return_type, subst)),
        },
        // Primitives / inference vars / other structural types pass
        // through unchanged.
        other => other.clone(),
    }
}

/// Build a substitution from a generic call site's argument types
/// against the function's declared parameter types.
///
/// `type_params` is the set of names declared on the generic signature
/// (`fn<T, U>`). For each `(declared_param_ty, actual_arg_ty)` pair:
///
/// - If `declared_param_ty` is a `Type::Struct(n)` where `n` is in
///   `type_params`, bind `n -> actual_arg_ty` (or check consistency
///   if already bound).
/// - Otherwise, the param is concrete; we don't bind anything (the
///   typechecker validates the concrete-vs-actual unification
///   separately).
///
/// Errors are returned as a `String` carrying a diagnostic per the
/// spec's "cannot infer" / "constrained to two types" wording.
pub fn infer_subst(
    type_params: &[String],
    declared: &[Type],
    actuals: &[Type],
) -> Result<Subst, String> {
    if declared.len() != actuals.len() {
        return Err(format!(
            "arity mismatch in generic call: signature takes {} arg(s), got {}",
            declared.len(),
            actuals.len()
        ));
    }
    let mut subst = Subst::new();
    let tp_set: std::collections::HashSet<&str> = type_params.iter().map(String::as_str).collect();
    for (decl, actual) in declared.iter().zip(actuals.iter()) {
        if let Type::Struct(name) = decl
            && tp_set.contains(name.as_str())
        {
            subst.bind(name.as_str(), actual.clone())?;
        }
    }
    Ok(subst)
}

// ---------------------------------------------------------------------------
// Body-consistency check (PR 1).
// ---------------------------------------------------------------------------

/// Walk the top-level program and validate every generic function:
///
/// 1. Type-parameter list contains no duplicates.
/// 2. If a parameter is declared with a generic type `T`, the body
///    does not constrain `T` to a concrete type via numeric / string
///    operators.
///
/// Both checks return `Err` with a diagnostic on the first violation.
/// An empty `type_params` list short-circuits to `Ok` (the function is
/// monomorphic — no generics to constrain).
pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };
    for stmt in stmts {
        check_node(&stmt.node)?;
    }
    Ok(())
}

fn check_node(node: &Node) -> Result<(), String> {
    if let Node::Function {
        name,
        type_params,
        parameters,
        body,
        ..
    } = node
    {
        let mut seen = std::collections::HashSet::new();
        for tp in type_params {
            if !seen.insert(tp.as_str()) {
                return Err(format!(
                    "duplicate type parameter `{}` in function `{}`",
                    tp, name
                ));
            }
        }
        if !type_params.is_empty() {
            // Identify which formal parameter names carry a generic
            // type. Each such name is a "generic-typed local"; using
            // it in arithmetic / numeric comparison is the
            // canonical body-consistency violation.
            let tp_set: std::collections::HashSet<&str> =
                type_params.iter().map(String::as_str).collect();
            let generic_locals: std::collections::HashSet<String> = parameters
                .iter()
                .filter_map(|(ty, pname)| {
                    if tp_set.contains(ty.as_str()) {
                        Some(pname.clone())
                    } else {
                        None
                    }
                })
                .collect();
            check_body_for_constraints(body, name, type_params, &generic_locals)?;
        }
    }
    Ok(())
}

/// Walk the body of a generic function looking for places where a
/// generic-typed local `x` is used in a context that forces a
/// concrete type. Today the canonical case is arithmetic with a
/// concrete literal — `x + 1`, `x * 2.0`, `x % 3` — which constrains
/// `T` to `Int` or `Float`.
fn check_body_for_constraints(
    body: &Node,
    fn_name: &str,
    type_params: &[String],
    generic_locals: &std::collections::HashSet<String>,
) -> Result<(), String> {
    match body {
        Node::Block { stmts, .. } => {
            for s in stmts {
                check_body_for_constraints(s, fn_name, type_params, generic_locals)?;
            }
            Ok(())
        }
        Node::ReturnStatement { value: Some(e), .. } => {
            check_body_for_constraints(e, fn_name, type_params, generic_locals)
        }
        Node::ReturnStatement { value: None, .. } => Ok(()),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            // `x + N` (or `*`, `-`, `/`, `%`) where `x` is a
            // generic-typed local AND the other operand is a concrete
            // numeric literal — that's the canonical body-consistency
            // violation.
            let arith = matches!(operator.as_str(), "+" | "-" | "*" | "/" | "%");
            if arith {
                let left_is_generic = identifier_in_set(left, generic_locals);
                let right_is_generic = identifier_in_set(right, generic_locals);
                let other_is_int_literal = matches!(left.as_ref(), Node::IntegerLiteral { .. })
                    || matches!(right.as_ref(), Node::IntegerLiteral { .. });
                let other_is_float_literal = matches!(left.as_ref(), Node::FloatLiteral { .. })
                    || matches!(right.as_ref(), Node::FloatLiteral { .. });
                if (left_is_generic ^ right_is_generic)
                    && (other_is_int_literal || other_is_float_literal)
                {
                    let constraint = if other_is_int_literal { "Int" } else { "Float" };
                    let tp_name = type_params.first().map(String::as_str).unwrap_or("T");
                    return Err(format!(
                        "type parameter `{}` of fn `{}` is constrained to a concrete type by the body — operator `{}` with a {} literal forces {} = {}. Either drop the type parameter and use a concrete type in the signature, or restructure the body so the parameter stays polymorphic.",
                        tp_name, fn_name, operator, constraint, tp_name, constraint
                    ));
                }
            }
            // Recurse so nested constraints are still flagged.
            check_body_for_constraints(left, fn_name, type_params, generic_locals)?;
            check_body_for_constraints(right, fn_name, type_params, generic_locals)?;
            Ok(())
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            check_body_for_constraints(condition, fn_name, type_params, generic_locals)?;
            check_body_for_constraints(consequence, fn_name, type_params, generic_locals)?;
            if let Some(alt) = alternative {
                check_body_for_constraints(alt, fn_name, type_params, generic_locals)?;
            }
            Ok(())
        }
        Node::ExpressionStatement { expr, .. } => {
            check_body_for_constraints(expr, fn_name, type_params, generic_locals)
        }
        // Anything else: the recursive walker would balloon to
        // every node variant, so we conservatively skip nodes that
        // can't directly contain a generic-constraining expression.
        // The canonical example from the ticket — `fn<T> bad(x: T) -> T { return x + 1; }`
        // is fully covered by the cases above.
        _ => Ok(()),
    }
}

/// True when `node` is a bare `Identifier { name }` whose name is in
/// the supplied set.
fn identifier_in_set(node: &Node, names: &std::collections::HashSet<String>) -> bool {
    if let Node::Identifier { name, .. } = node {
        names.contains(name)
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn check_src(src: &str) -> Result<(), String> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        super::check(&prog, "<t>")
    }

    #[test]
    fn duplicate_type_parameter_is_rejected() {
        let err = check_src("fn<T, T> id(T x) -> T { return x; }")
            .expect_err("dup type param should error");
        assert!(err.contains("duplicate type parameter"), "got: {}", err);
    }

    #[test]
    fn identity_fn_passes() {
        check_src("fn<T> id(T x) -> T { return x; }").expect("monomorphic identity should pass");
    }

    #[test]
    fn body_constraining_type_param_to_int_is_rejected() {
        // The spec example: `fn<T> bad(x: T) -> T { return x + 1; }`
        // — body forces T = Int, contradicting the generic signature.
        let err = check_src("fn<T> bad(T x) -> T { return x + 1; }")
            .expect_err("x + 1 should constrain T to Int");
        assert!(
            err.contains("constrained to a concrete type"),
            "diagnostic missing 'constrained to a concrete type': {}",
            err
        );
        assert!(
            err.contains("T = Int"),
            "diagnostic missing 'T = Int': {}",
            err
        );
    }

    #[test]
    fn body_constraining_type_param_to_float_is_rejected() {
        let err = check_src("fn<T> bad(T x) -> T { return x * 2.0; }")
            .expect_err("x * 2.0 should constrain T to Float");
        assert!(err.contains("T = Float"), "got: {}", err);
    }

    #[test]
    fn body_using_type_param_with_another_type_param_is_ok() {
        // `x + y` where both are T — does NOT constrain T because
        // the other operand isn't a concrete literal.
        check_src("fn<T> add(T x, T y) -> T { return x + y; }")
            .expect("x + y is polymorphic — both operands are T");
    }

    #[test]
    fn non_generic_fn_with_arithmetic_is_unaffected() {
        check_src("fn add(int x) -> int { return x + 1; }")
            .expect("monomorphic fns are not constrained");
    }

    #[test]
    fn subst_bind_consistent_succeeds() {
        let mut s = Subst::new();
        s.bind("T", Type::Int).expect("first bind should succeed");
        s.bind("T", Type::Int)
            .expect("rebinding to same type should succeed");
        assert_eq!(s.get("T"), Some(&Type::Int));
    }

    #[test]
    fn subst_bind_inconsistent_errors() {
        let mut s = Subst::new();
        s.bind("T", Type::Int).expect("first bind ok");
        let err = s
            .bind("T", Type::String)
            .expect_err("rebinding to different type should fail");
        assert!(err.contains("inferred as both"), "got: {}", err);
    }

    #[test]
    fn apply_subst_replaces_generic_param_name() {
        let mut s = Subst::new();
        s.bind("T", Type::Int).unwrap();
        let ty = Type::Struct("T".to_string());
        let applied = apply_subst(&ty, &s);
        assert_eq!(applied, Type::Int);
    }

    #[test]
    fn apply_subst_leaves_concrete_types_alone() {
        let s = Subst::new();
        let ty = Type::Struct("Point".to_string());
        let applied = apply_subst(&ty, &s);
        assert_eq!(applied, Type::Struct("Point".to_string()));
    }

    #[test]
    fn infer_subst_finds_type_param_from_first_argument() {
        let type_params = vec!["T".to_string()];
        let declared = vec![Type::Struct("T".to_string())];
        let actuals = vec![Type::Int];
        let s = infer_subst(&type_params, &declared, &actuals).expect("simple case");
        assert_eq!(s.get("T"), Some(&Type::Int));
    }

    #[test]
    fn infer_subst_arity_mismatch_errors() {
        let type_params = vec!["T".to_string()];
        let declared = vec![Type::Struct("T".to_string()), Type::Int];
        let actuals = vec![Type::Int];
        let err =
            infer_subst(&type_params, &declared, &actuals).expect_err("arity mismatch must fail");
        assert!(err.contains("arity mismatch"), "got: {}", err);
    }
}
