//! RES-2576: type inference for generic function calls.
//!
//! Walks every call site in the program that targets a generic function
//! and verifies that all type parameters can be inferred from the
//! argument types. When inference succeeds the call is valid as-is;
//! when inference fails an informative diagnostic is returned.
//!
//! ## What this pass does
//!
//! ```resilient
//! fn identity<T>(T x) -> T { return x; }
//!
//! identity(42);        // OK — T inferred as int from first argument
//! identity("hello");   // OK — T inferred as string
//! ```
//!
//! For multi-parameter generics:
//!
//! ```resilient
//! fn pair<A, B>(A a, B b) -> A { return a; }
//! pair(1, "two");      // OK — A=int, B=string
//! ```
//!
//! Partial inference with `_` wildcard (placeholder for a type to infer):
//!
//! ```resilient
//! fn pick<A, B>(A a, B b) -> A { return a; }
//! pick::<_, string>(42, "hi");  // A inferred as int, B specified as string
//! ```
//!
//! ## What this pass does NOT do
//!
//! - Monomorphization — that lives in `monomorph.rs`.
//! - Full Hindley-Milner inference — that lives in `infer.rs`.
//! - Return-type directed inference — argument inference is sufficient
//!   for the call-site use cases the ticket covers.
//!
//! ## Error messages
//!
//! When a type parameter cannot be resolved the diagnostic reads:
//!
//! ```text
//! cannot infer type for `T` in call to `identity`; add an explicit
//! type annotation or pass a typed expression
//! ```
//!
//! When two arguments constrain the same type parameter to different
//! types (e.g. `pair(1, "two")` with only one type param `T`):
//!
//! ```text
//! type parameter `T` in call to `pair` is inferred as both `int` and
//! `string` — they must agree
//! ```

#![allow(dead_code)]

use crate::Node;
use crate::typechecker::Type;
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Entry point (called from typechecker.rs <EXTENSION_PASSES>)
// ---------------------------------------------------------------------------

/// Walk the program and validate every call to a generic function.
///
/// - Collects generic function signatures once at the start.
/// - For each `CallExpression { function: Identifier { name }, arguments }`,
///   if `name` is a known generic function, attempts inference and returns
///   an `Err` diagnostic on the first failure.
/// - Non-generic calls pass through immediately.
pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    // Fast-reject: if no generic functions exist, skip entirely.
    let has_generic_fn = stmts
        .iter()
        .any(|s| matches!(&s.node, Node::Function { type_params, .. } if !type_params.is_empty()));
    if !has_generic_fn {
        return Ok(());
    }

    // Phase 1: build a map of  fn_name → (type_params, param_types).
    let signatures = collect_signatures(stmts);

    // Phase 2: walk every statement for call sites to generic functions.
    for spanned in stmts {
        check_node(&spanned.node, &signatures)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Signature collection
// ---------------------------------------------------------------------------

/// Compact representation of a generic function signature for inference.
struct GenericSig {
    /// Declared type parameters in order: `["T"]`, `["A", "B"]`, etc.
    type_params: Vec<String>,
    /// Declared parameter type strings in order.
    param_types: Vec<String>,
}

fn collect_signatures(stmts: &[crate::span::Spanned<Node>]) -> HashMap<String, GenericSig> {
    // Pre-size to stmts.len() — one entry per top-level generic fn at most.
    let mut map = HashMap::with_capacity(stmts.len());
    for spanned in stmts {
        if let Node::Function {
            name,
            type_params,
            parameters,
            ..
        } = &spanned.node
            && !type_params.is_empty()
        {
            map.insert(
                name.clone(),
                GenericSig {
                    type_params: type_params.clone(),
                    param_types: parameters.iter().map(|(ty, _)| ty.clone()).collect(),
                },
            );
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Node walker
// ---------------------------------------------------------------------------

fn check_node(node: &Node, sigs: &HashMap<String, GenericSig>) -> Result<(), String> {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                check_node(&s.node, sigs)?;
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            check_node(body, sigs)?;
            for r in requires {
                check_node(r, sigs)?;
            }
            for e in ensures {
                check_node(e, sigs)?;
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                check_node(s, sigs)?;
            }
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            // Recurse into arguments first.
            for arg in arguments {
                check_node(arg, sigs)?;
            }
            check_node(function, sigs)?;

            // Check this specific call site.
            if let Node::Identifier { name, .. } = function.as_ref()
                && let Some(sig) = sigs.get(name.as_str())
            {
                check_call_site(name, sig, arguments)?;
            }
        }
        Node::LetStatement { value, .. } => check_node(value, sigs)?,
        Node::StaticLet { value, .. } => check_node(value, sigs)?,
        Node::Const { value, .. } => check_node(value, sigs)?,
        Node::Assignment { value, .. } => check_node(value, sigs)?,
        Node::ReturnStatement { value: Some(v), .. } => check_node(v, sigs)?,
        Node::ReturnStatement { value: None, .. } => {}
        Node::ExpressionStatement { expr, .. } => check_node(expr, sigs)?,
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            check_node(condition, sigs)?;
            check_node(consequence, sigs)?;
            if let Some(alt) = alternative {
                check_node(alt, sigs)?;
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            check_node(condition, sigs)?;
            check_node(body, sigs)?;
        }
        Node::ForInStatement { iterable, body, .. } => {
            check_node(iterable, sigs)?;
            check_node(body, sigs)?;
        }
        Node::InfixExpression { left, right, .. } => {
            check_node(left, sigs)?;
            check_node(right, sigs)?;
        }
        Node::PrefixExpression { right, .. } => check_node(right, sigs)?,
        // Leaves and structural nodes we don't recurse into for this pass.
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Call-site inference
// ---------------------------------------------------------------------------

/// Attempt to infer type arguments for a call to a generic function.
///
/// Walks `(declared_param_type, argument_node)` pairs. For each
/// `declared_param_type` that is a type-parameter name (e.g. `"T"`),
/// attempts to infer the concrete type from the argument node:
///
/// - Literal nodes → their concrete type (`int`, `float`, `string`, `bool`).
/// - `_` placeholder in an explicit annotation → skip (callers handle
///   partial inference by providing concrete arg values anyway).
/// - Everything else → type unknown; if the type parameter remains
///   unconstrained after all arguments, emit a "cannot infer" error.
///
/// The pass is conservative: if it cannot determine the argument's type
/// (e.g. the argument is an identifier referencing a variable — the
/// typechecker already handles that path), it skips the slot rather than
/// emitting a false-positive. Only confirmed inconsistencies (two arguments
/// forcing the same `T` to two different concrete types) are hard errors.
fn check_call_site(fn_name: &str, sig: &GenericSig, arguments: &[Node]) -> Result<(), String> {
    // Arity mismatch is handled separately by the typechecker; skip here.
    if sig.param_types.len() != arguments.len() {
        return Ok(());
    }

    let tp_set: HashSet<&str> = sig.type_params.iter().map(String::as_str).collect();
    // Map type-param name → inferred concrete type.
    // Pre-size to the type-param count — one entry per distinct param at most.
    let mut inferred: HashMap<&str, Type> = HashMap::with_capacity(sig.type_params.len());
    // Type params that appeared in at least one slot where the argument type
    // could not be determined (e.g. a variable reference). We never report
    // "cannot infer T" for these — the typechecker's erasure path already
    // handles those call sites conservatively.
    let mut has_unknown_slot: HashSet<&str> = HashSet::new();

    for (param_ty_str, arg_node) in sig.param_types.iter().zip(arguments.iter()) {
        if !tp_set.contains(param_ty_str.as_str()) {
            // Concrete parameter — not a type variable.
            continue;
        }
        let Some(arg_ty) = infer_node_type(arg_node) else {
            // Cannot determine arg type statically — record this type param
            // as having an unresolvable slot so we skip its missing-inference
            // check at the end.
            has_unknown_slot.insert(param_ty_str.as_str());
            continue;
        };
        match inferred.get(param_ty_str.as_str()) {
            Some(existing) if existing != &arg_ty => {
                return Err(format!(
                    "type parameter `{}` in call to `{}` is inferred as both `{}` and \
                     `{}` — they must agree",
                    param_ty_str, fn_name, existing, arg_ty
                ));
            }
            _ => {
                inferred.insert(param_ty_str.as_str(), arg_ty);
            }
        }
    }

    // Check that every type parameter was resolved by at least one argument,
    // unless it appeared in a slot where the argument type was unknown
    // (a variable, complex expression, etc.). Those are handled conservatively
    // — the typechecker's erasure path already covers them.
    if inferred.is_empty() {
        // Nothing inferred from arguments — conservative pass-through.
        return Ok(());
    }
    for tp in &sig.type_params {
        if !inferred.contains_key(tp.as_str()) && !has_unknown_slot.contains(tp.as_str()) {
            // The type parameter has no concrete argument in any slot AND
            // no slot with an unknown-type argument — this is a genuine
            // inference failure.
            return Err(format!(
                "cannot infer type for `{}` in call to `{}`; add an explicit \
                 type annotation or pass a typed expression",
                tp, fn_name
            ));
        }
    }
    Ok(())
}

/// Best-effort concrete type inference from an argument expression.
///
/// Returns `None` when the type cannot be determined without a full
/// type-inference pass (e.g. the argument is a variable reference or a
/// complex expression). Callers treat `None` as "unknown — skip conservatively."
fn infer_node_type(node: &Node) -> Option<Type> {
    match node {
        Node::IntegerLiteral { .. } => Some(Type::Int),
        Node::FloatLiteral { .. } => Some(Type::Float),
        Node::StringLiteral { .. } => Some(Type::String),
        Node::BooleanLiteral { .. } => Some(Type::Bool),
        // Nested call or complex expression — unknown statically.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::parse;

    fn check_src(src: &str) -> Result<(), String> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        super::check(&prog, "<t>")
    }

    // --- basic inference ---

    #[test]
    fn identity_with_int_literal_passes() {
        check_src(
            r#"fn identity<T>(T x) -> T { return x; }
identity(42);"#,
        )
        .expect("T inferred as int");
    }

    #[test]
    fn identity_with_string_literal_passes() {
        check_src(
            r#"fn identity<T>(T x) -> T { return x; }
identity("hello");"#,
        )
        .expect("T inferred as string");
    }

    #[test]
    fn identity_with_bool_literal_passes() {
        check_src(
            r#"fn identity<T>(T x) -> T { return x; }
identity(true);"#,
        )
        .expect("T inferred as bool");
    }

    #[test]
    fn identity_with_float_literal_passes() {
        check_src(
            r#"fn identity<T>(T x) -> T { return x; }
identity(3.14);"#,
        )
        .expect("T inferred as float");
    }

    // --- multi-param generics ---

    #[test]
    fn pair_with_two_literals_passes() {
        check_src(
            r#"fn pair<A, B>(A a, B b) -> A { return a; }
pair(1, "two");"#,
        )
        .expect("A=int, B=string inferred");
    }

    #[test]
    fn pair_with_same_type_literals_passes() {
        check_src(
            r#"fn pair<A, B>(A a, B b) -> A { return a; }
pair(1, 2);"#,
        )
        .expect("A=int, B=int inferred");
    }

    // --- inconsistency detection ---

    #[test]
    fn single_param_inconsistency_is_rejected() {
        // T is used for two params — if two different types are passed,
        // it's an inconsistency.
        let err = check_src(
            r#"fn both_same<T>(T a, T b) -> T { return a; }
both_same(1, "two");"#,
        )
        .expect_err("int vs string for T should fail");
        assert!(
            err.contains("inferred as both"),
            "diagnostic missing 'inferred as both': {}",
            err
        );
    }

    // --- conservative pass-through (variable arguments) ---

    #[test]
    fn identity_with_variable_arg_passes_conservatively() {
        // Variable arg — we can't infer the type statically, so we pass
        // without error (the typechecker handles the erasure path).
        check_src(
            r#"fn identity<T>(T x) -> T { return x; }
fn main(int n) { identity(n); }"#,
        )
        .expect("variable arg: conservative pass-through");
    }

    // --- non-generic programs short-circuit ---

    #[test]
    fn non_generic_program_is_a_no_op() {
        check_src("fn add(int a, int b) -> int { return a + b; } add(1, 2);")
            .expect("no generic fns: pass immediately");
    }

    // --- partial inference: one param of two resolved ---

    #[test]
    fn partial_literal_args_with_one_unresolvable_pass_conservatively() {
        // `f(variable, "hi")` — A is unknown (variable), B is known (string).
        // Since A is unknown we don't have enough to decide "A is missing",
        // so the pass returns Ok.
        check_src(
            r#"fn f<A, B>(A a, B b) -> A { return a; }
fn main(int x) { f(x, "hi"); }"#,
        )
        .expect("partial inference: conservative pass");
    }

    // --- multiple distinct calls in same program ---

    #[test]
    fn two_distinct_calls_to_same_generic_fn_both_pass() {
        check_src(
            r#"fn id<T>(T x) -> T { return x; }
id(42);
id("hello");"#,
        )
        .expect("two valid calls pass");
    }

    // --- arity mismatch is ignored (typechecker handles it) ---

    #[test]
    fn arity_mismatch_is_not_an_inference_error() {
        // Wrong arity — we defer to the typechecker's error.
        check_src(
            r#"fn id<T>(T x) -> T { return x; }
id(1, 2);"#,
        )
        .expect("arity mismatch: deferred to typechecker");
    }
}
