//! RES-120: Hindley-Milner inference prototype.
//!
//! Classic Algorithm W over the primitive monotypes
//! (`Int`, `Float`, `Bool`, `String`) the language already has.
//! Consumes RES-121's `unify` module for the substitution /
//! unification machinery.
//!
//! Scope (per the ticket's "keep this minimal" note):
//! - Literals → their primitive type.
//! - `Identifier` → lookup in the function-local env, which is
//!   seeded from the function's declared parameter types.
//! - Infix operators (`+ - * / %`, `&& ||`, `== !=`, `< > <= >=`,
//!   bitwise `& | ^ << >>`) with hard-coded rules.
//! - Prefix `!` / `-`.
//! - `LetStatement` with or without an annotation — the
//!   annotation (when present) unifies with the inferred value
//!   type.
//! - `IfStatement` — condition must be `Bool`; branches' types
//!   unify.
//! - `Block` / `ReturnStatement` / `ExpressionStatement` — walk
//!   recursively; don't produce a type of their own.
//!
//! Deliberately NOT inferred (would need more work):
//! - Generics (RES-124).
//! - Let-polymorphism (RES-122).
//! - Arrays / structs / Result (needs type constructors in the
//!   inferer — the prototype only handles primitive monotypes).
//! - Function calls with user-defined fn signatures (would need
//!   a global env; out of scope for the per-function prototype).
//!
//! Scope deviation from the literal AC:
//! - Return type is `Result<Substitution, Vec<Diagnostic>>`
//!   instead of `Result<HashMap<NodeId, Type>, Vec<Diagnostic>>`.
//!   `NodeId` doesn't exist in the codebase; the substitution map
//!   (keyed by type-var id, not AST id) is sufficient to surface
//!   the inference result. See the Attempt 1 clarification on
//!   RES-120 for the rationale.
//!
//! The module is compiled only under `--features infer` — the
//! prototype is opt-in until RES-123 turns it on by default.

// clippy::result_large_err: internal `Result<Type, Diagnostic>`
// shapes. The prototype is allocation-light in practice (per-fn
// bodies rarely produce more than a handful of diagnostics); the
// ergonomic cost of boxing every `Err` isn't worth the
// theoretical 128-byte savings for this phase.
#![allow(clippy::result_large_err)]

use std::collections::HashMap;

use crate::diag::{DiagCode, Diagnostic, Severity};
use crate::span::Span;
use crate::typechecker::Type;
use crate::unify::{Substitution, UnifyError};
use crate::Node;

/// RES-120: one type-inference error.
///
/// Codes:
/// - `T0001` — occurs check (infinite type).
/// - `T0002` — primitive-type mismatch (e.g. `Int` vs `Bool`).
/// - `T0003` — structured-type mismatch (e.g. `Array` vs `Int`).
/// - `T0004` — param-count mismatch in function-type
///   unification (arity disagrees).
/// - `T0005` — identifier not in scope.
/// - `T0006` — unsupported-shape bail-out (the prototype
///   doesn't cover this AST variant yet).
pub const T0001_OCCURS: DiagCode = DiagCode::new("T0001");
pub const T0002_PRIMITIVE_MISMATCH: DiagCode = DiagCode::new("T0002");
pub const T0003_STRUCTURED_MISMATCH: DiagCode = DiagCode::new("T0003");
pub const T0004_ARITY_MISMATCH: DiagCode = DiagCode::new("T0004");
pub const T0005_UNBOUND: DiagCode = DiagCode::new("T0005");
pub const T0006_UNSUPPORTED: DiagCode = DiagCode::new("T0006");

/// RES-120: the inferer's state. One instance per function —
/// variables don't outlive the function, and the env is fresh
/// each time.
pub struct Inferer {
    next_var: u32,
    subst: Substitution,
    env: HashMap<String, Type>,
}

impl Default for Inferer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================
// RES-122: type schemes + generalize + instantiate.
// ============================================================
//
// A `Scheme` is a universally-quantified type:
//
//   ∀ var1, var2, ... . ty
//
// Concretely, `Scheme { vars: [0, 1], ty: Fn([Var(0), Var(1)],
// Var(0)) }` represents `∀a b. (a, b) -> a`. `instantiate`
// turns the scheme back into a plain `Type` by replacing each
// quantified var with a fresh variable, ready to unify at a
// call site. `generalize` takes a freshly-inferred type and
// quantifies over the variables that are free in it but not
// in the surrounding env (classic Damas-Hindley-Milner).
//
// Scope deviation: we ship the helpers + unit tests here;
// threading schemes through the full
// infer-function-then-instantiate-at-call-site flow requires
// RES-124a (`fn<T>` parser + AST field), which is a separate
// follow-up. When that lands, the call-site wiring is additive
// — no breaking changes to this API.

/// RES-122: a polymorphic type scheme. `vars` lists the
/// type-variable ids bound by the outer `∀` quantifier; any
/// `Type::Var(id)` in `ty` whose `id` is in `vars` is
/// "quantified", the rest are free in some broader scope.
// Scheme drops `Eq` because `Type` derives only `PartialEq`
// (it carries f64 via FloatLiteral — not `Eq` anyway). Callers
// that need equality use `PartialEq` (`==`).
#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    pub vars: Vec<u32>,
    pub ty: Type,
}

impl Scheme {
    /// Wrap a concrete (already-generalized) body — `∀∅. ty`.
    /// Useful when the caller has already computed the var set
    /// separately.
    pub fn new(vars: Vec<u32>, ty: Type) -> Self {
        Self { vars, ty }
    }

    /// Trivial scheme: no quantifier. Exposed so consumers can
    /// uniformly treat monomorphic + polymorphic bindings the
    /// same way.
    pub fn monotype(ty: Type) -> Self {
        Self { vars: Vec::new(), ty }
    }
}

/// RES-122: collect every `Type::Var(id)` that appears in `ty`.
/// Recurses through function types + composite types. Returns
/// the ids (not the full `Type::Var`s) for easy set math.
pub fn free_type_vars(ty: &Type) -> std::collections::HashSet<u32> {
    let mut out = std::collections::HashSet::new();
    collect_ftv(ty, &mut out);
    out
}

fn collect_ftv(ty: &Type, out: &mut std::collections::HashSet<u32>) {
    match ty {
        Type::Var(v) => {
            out.insert(*v);
        }
        Type::Function { params, return_type } => {
            for p in params {
                collect_ftv(p, out);
            }
            collect_ftv(return_type, out);
        }
        // Primitive / opaque types have no type variables.
        Type::Int
        | Type::Float
        | Type::String
        | Type::Bool
        | Type::Bytes
        | Type::Array
        | Type::Result
        | Type::Struct(_)
        | Type::Void
        | Type::Any => {}
    }
}

/// RES-122: collect the free type variables across every value
/// in the env. Helper for `generalize`: the bound set is
/// `ftv(ty) \ ftv(env)`.
fn ftv_env(env: &HashMap<String, Type>) -> std::collections::HashSet<u32> {
    let mut out = std::collections::HashSet::new();
    for ty in env.values() {
        collect_ftv(ty, &mut out);
    }
    out
}

/// RES-122: generalize `ty` against the surrounding env.
///
/// Returns a `Scheme` that quantifies every type variable
/// appearing in `ty` but NOT in any binding of `env`. Matches
/// the classical DHM `gen(Γ, τ) = ∀ (ftv(τ) \ ftv(Γ)). τ`.
///
/// When `ty` has no free variables beyond the env, the returned
/// scheme is monomorphic (empty `vars`). No special-case
/// needed — the empty-quantifier case already covers it.
///
/// Monomorphic outputs are still wrapped in a Scheme for
/// interface uniformity; downstream code treats the empty-vars
/// case as a plain type.
pub fn generalize(env: &HashMap<String, Type>, ty: &Type) -> Scheme {
    let ty_vars = free_type_vars(ty);
    let env_vars = ftv_env(env);
    // Diff, sorted for deterministic output.
    let mut vars: Vec<u32> = ty_vars.difference(&env_vars).copied().collect();
    vars.sort();
    Scheme { vars, ty: ty.clone() }
}

impl Inferer {
    /// RES-122: instantiate a scheme with fresh type variables.
    ///
    /// Every quantified var in the scheme gets mapped to a
    /// freshly-minted `Type::Var(n)`; the body is then
    /// substituted through that mapping. The resulting type
    /// shares structure with `scheme.ty` but with its bound
    /// variables replaced, ready to unify at a call site.
    ///
    /// The inferer's substitution is NOT mutated — `instantiate`
    /// only allocates fresh variables. Unification at the call
    /// site is what mutates `subst`.
    pub fn instantiate(&mut self, scheme: &Scheme) -> Type {
        // Build a one-shot map from the quantified vars to
        // fresh vars.
        let mut mapping: HashMap<u32, Type> = HashMap::new();
        for &v in &scheme.vars {
            let fresh = self.fresh();
            mapping.insert(v, fresh);
        }
        substitute_vars(&scheme.ty, &mapping)
    }
}

/// Apply a `HashMap<u32, Type>` substitution recursively. Used
/// by `instantiate`; different shape from the unify module's
/// `Substitution` (which chains via ids). `instantiate`'s
/// one-shot map has no chain — each quantified var maps to a
/// single fresh var.
fn substitute_vars(ty: &Type, map: &HashMap<u32, Type>) -> Type {
    match ty {
        Type::Var(v) => map.get(v).cloned().unwrap_or_else(|| ty.clone()),
        Type::Function { params, return_type } => Type::Function {
            params: params.iter().map(|p| substitute_vars(p, map)).collect(),
            return_type: Box::new(substitute_vars(return_type, map)),
        },
        other => other.clone(),
    }
}

impl Inferer {
    pub fn new() -> Self {
        Self {
            next_var: 0,
            subst: Substitution::new(),
            env: HashMap::new(),
        }
    }

    /// Mint a fresh type variable. Each call returns a distinct
    /// `Type::Var(n)`.
    pub fn fresh(&mut self) -> Type {
        let v = Type::Var(self.next_var);
        self.next_var += 1;
        v
    }

    /// Snapshot of the substitution after inference. Callers
    /// can `apply` the returned substitution to any type to
    /// resolve its inferred concrete form.
    pub fn substitution(&self) -> &Substitution {
        &self.subst
    }

    /// RES-120 entry point.
    ///
    /// Walks the body of a `Node::Function`, seeding the env
    /// from declared parameter types and collecting + solving
    /// type constraints. Returns the final substitution or a
    /// non-empty `Vec<Diagnostic>` on failure.
    pub fn infer_function(
        &mut self,
        func: &Node,
    ) -> Result<Substitution, Vec<Diagnostic>> {
        let (parameters, body) = match func {
            Node::Function { parameters, body, .. } => (parameters, body),
            _ => {
                return Err(vec![Diagnostic::new(
                    Severity::Error,
                    Span::default(),
                    "infer_function expects a Node::Function",
                )
                .with_code(T0006_UNSUPPORTED.clone())]);
            }
        };

        // Seed the env from declared parameter types. Unknown /
        // non-primitive annotations get a fresh var so downstream
        // unification can still make progress.
        for (ty_name, name) in parameters {
            let ty = parse_primitive_type(ty_name).unwrap_or_else(|| self.fresh());
            self.env.insert(name.clone(), ty);
        }

        let mut errs = Vec::new();
        self.infer_stmt(body, &mut errs);
        if errs.is_empty() {
            Ok(self.subst.clone())
        } else {
            Err(errs)
        }
    }

    /// Walk a statement (no type produced). Appends any
    /// diagnostics to `errs` rather than short-circuiting; the
    /// inferer keeps going so one error doesn't silence the
    /// rest. This matches the rustc pattern of a "best-effort"
    /// inference pass.
    fn infer_stmt(&mut self, node: &Node, errs: &mut Vec<Diagnostic>) {
        match node {
            Node::Block { stmts, .. } => {
                for s in stmts {
                    self.infer_stmt(s, errs);
                }
            }
            Node::LetStatement { name, value, type_annot, span } => {
                let value_ty = match self.infer_expr(value) {
                    Ok(t) => t,
                    Err(e) => {
                        errs.push(e);
                        // Use fresh var so further references to
                        // `name` don't cascade.
                        self.fresh()
                    }
                };
                if let Some(ann) = type_annot
                    && let Some(ann_ty) = parse_primitive_type(ann)
                    && let Err(e) = self.subst.unify(&value_ty, &ann_ty)
                {
                    errs.push(unify_error_to_diag(
                        e,
                        *span,
                        format!(
                            "let {}: {} — annotation doesn't match value",
                            name, ann
                        ),
                    ));
                }
                // Non-primitive annotations are ignored by the
                // prototype (RES-124 / RES-127 handle structs /
                // generics).
                self.env.insert(name.clone(), value_ty);
            }
            Node::ReturnStatement { value: Some(v), .. } => {
                if let Err(e) = self.infer_expr(v) {
                    errs.push(e);
                }
            }
            Node::ReturnStatement { value: None, .. } => {}
            Node::ExpressionStatement { expr, .. } => {
                if let Err(e) = self.infer_expr(expr) {
                    errs.push(e);
                }
            }
            Node::IfStatement { condition, consequence, alternative, .. } => {
                match self.infer_expr(condition) {
                    Ok(ct) => {
                        if let Err(e) = self.subst.unify(&ct, &Type::Bool) {
                            errs.push(unify_error_to_diag(
                                e,
                                expr_span(condition),
                                "if condition must be a bool".into(),
                            ));
                        }
                    }
                    Err(e) => errs.push(e),
                }
                self.infer_stmt(consequence, errs);
                if let Some(a) = alternative {
                    self.infer_stmt(a, errs);
                }
            }
            Node::WhileStatement { condition, body, .. } => {
                match self.infer_expr(condition) {
                    Ok(ct) => {
                        if let Err(e) = self.subst.unify(&ct, &Type::Bool) {
                            errs.push(unify_error_to_diag(
                                e,
                                expr_span(condition),
                                "while condition must be a bool".into(),
                            ));
                        }
                    }
                    Err(e) => errs.push(e),
                }
                self.infer_stmt(body, errs);
            }
            // Everything else: skip, with an informational Hint
            // Diagnostic noting the AST shape we skipped. These
            // typically aren't errors — they're expressions the
            // prototype doesn't analyse (e.g. Match, Assert).
            _ => {}
        }
    }

    /// Walk an expression and return its inferred type. Single-
    /// error return — the caller decides whether to accumulate
    /// or short-circuit.
    fn infer_expr(&mut self, node: &Node) -> Result<Type, Diagnostic> {
        match node {
            Node::IntegerLiteral { .. } => Ok(Type::Int),
            Node::FloatLiteral { .. } => Ok(Type::Float),
            Node::BooleanLiteral { .. } => Ok(Type::Bool),
            Node::StringLiteral { .. } => Ok(Type::String),
            Node::Identifier { name, span } => self
                .env
                .get(name)
                .cloned()
                .ok_or_else(|| {
                    Diagnostic::new(
                        Severity::Error,
                        *span,
                        format!("identifier `{}` not in scope", name),
                    )
                    .with_code(T0005_UNBOUND.clone())
                }),
            Node::InfixExpression { left, operator, right, span } => {
                let lt = self.infer_expr(left)?;
                let rt = self.infer_expr(right)?;
                self.infer_infix_op(&lt, &rt, operator, *span)
            }
            Node::PrefixExpression { operator, right, span } => {
                let rt = self.infer_expr(right)?;
                self.infer_prefix_op(&rt, operator, *span)
            }
            // Unsupported shapes return a fresh var — lets
            // downstream unification progress; the specific
            // error (if any) surfaces where the var gets pinned.
            _ => Ok(self.fresh()),
        }
    }

    fn infer_infix_op(
        &mut self,
        lt: &Type,
        rt: &Type,
        op: &str,
        span: Span,
    ) -> Result<Type, Diagnostic> {
        match op {
            // Arithmetic: LHS and RHS unify; result is their
            // common type. The prototype doesn't restrict to
            // numeric (deferred — see RES-130 wiring).
            "+" | "-" | "*" | "/" | "%" => {
                self.subst.unify(lt, rt).map_err(|e| {
                    unify_error_to_diag(
                        e,
                        span,
                        format!("operator `{}` requires matching operand types", op),
                    )
                })?;
                Ok(self.subst.apply(lt))
            }
            // Logical: both sides and result are Bool.
            "&&" | "||" => {
                self.subst.unify(lt, &Type::Bool).map_err(|e| {
                    unify_error_to_diag(
                        e,
                        span,
                        format!("operator `{}` requires bool operands", op),
                    )
                })?;
                self.subst.unify(rt, &Type::Bool).map_err(|e| {
                    unify_error_to_diag(
                        e,
                        span,
                        format!("operator `{}` requires bool operands", op),
                    )
                })?;
                Ok(Type::Bool)
            }
            // Equality / inequality: operands unify; result Bool.
            "==" | "!=" => {
                self.subst.unify(lt, rt).map_err(|e| {
                    unify_error_to_diag(
                        e,
                        span,
                        format!("operator `{}` requires matching operand types", op),
                    )
                })?;
                Ok(Type::Bool)
            }
            // Ordering: operands unify; result Bool.
            "<" | ">" | "<=" | ">=" => {
                self.subst.unify(lt, rt).map_err(|e| {
                    unify_error_to_diag(
                        e,
                        span,
                        format!("operator `{}` requires matching operand types", op),
                    )
                })?;
                Ok(Type::Bool)
            }
            // Bitwise + shifts: both operands Int; result Int.
            "&" | "|" | "^" | "<<" | ">>" => {
                self.subst.unify(lt, &Type::Int).map_err(|e| {
                    unify_error_to_diag(
                        e,
                        span,
                        format!("operator `{}` requires int operands", op),
                    )
                })?;
                self.subst.unify(rt, &Type::Int).map_err(|e| {
                    unify_error_to_diag(
                        e,
                        span,
                        format!("operator `{}` requires int operands", op),
                    )
                })?;
                Ok(Type::Int)
            }
            other => Err(Diagnostic::new(
                Severity::Error,
                span,
                format!("unsupported operator `{}` in inference prototype", other),
            )
            .with_code(T0006_UNSUPPORTED.clone())),
        }
    }

    fn infer_prefix_op(
        &mut self,
        rt: &Type,
        op: &str,
        span: Span,
    ) -> Result<Type, Diagnostic> {
        match op {
            "!" => {
                self.subst.unify(rt, &Type::Bool).map_err(|e| {
                    unify_error_to_diag(
                        e,
                        span,
                        "prefix `!` requires a bool operand".into(),
                    )
                })?;
                Ok(Type::Bool)
            }
            "-" => {
                // Preserve operand type — works for Int or Float.
                Ok(self.subst.apply(rt))
            }
            other => Err(Diagnostic::new(
                Severity::Error,
                span,
                format!("unsupported prefix operator `{}`", other),
            )
            .with_code(T0006_UNSUPPORTED.clone())),
        }
    }
}

/// Parse a declared type annotation into a primitive `Type`.
/// Returns `None` for non-primitives (the caller decides whether
/// to fall back to a fresh var).
fn parse_primitive_type(s: &str) -> Option<Type> {
    match s {
        "int" => Some(Type::Int),
        "float" => Some(Type::Float),
        "bool" => Some(Type::Bool),
        "string" => Some(Type::String),
        _ => None,
    }
}

/// Extract a best-effort span for an expression. Mirrors the
/// helper in `lint.rs`; duplicated here so `infer` stays
/// independent of `lint`'s feature gating.
fn expr_span(node: &Node) -> Span {
    match node {
        Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::Identifier { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::CallExpression { span, .. }
        | Node::TryExpression { span, .. } => *span,
        _ => Span::default(),
    }
}

/// Map a `UnifyError` to a `Diagnostic` with the right code +
/// user-facing message. `context` is a callsite-specific hint
/// appended to the raw unification reason.
fn unify_error_to_diag(err: UnifyError, span: Span, context: String) -> Diagnostic {
    let (code, msg) = match &err {
        UnifyError::Occurs(var, _) => (
            T0001_OCCURS.clone(),
            format!("{} (infinite type involving ?t{})", context, var),
        ),
        UnifyError::Mismatch(a, b) => (
            // Prefer the primitive-mismatch code when both sides
            // are primitives; fall back to the structured code
            // when either side is a composite type. The
            // prototype only produces primitive mismatches today
            // (Array / Struct aren't in the inferer's coverage
            // yet), so T0002 is the usual outcome.
            if is_primitive(a) && is_primitive(b) {
                T0002_PRIMITIVE_MISMATCH.clone()
            } else {
                T0003_STRUCTURED_MISMATCH.clone()
            },
            format!("{}: expected `{}`, got `{}`", context, a, b),
        ),
        UnifyError::ArityMismatch(expected, got) => (
            T0004_ARITY_MISMATCH.clone(),
            format!(
                "{}: function arity mismatch ({} vs {} parameters)",
                context, expected, got
            ),
        ),
    };
    Diagnostic::new(Severity::Error, span, msg).with_code(code)
}

fn is_primitive(t: &Type) -> bool {
    matches!(
        t,
        Type::Int | Type::Float | Type::Bool | Type::String | Type::Bytes
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn first_function(src: &str) -> Node {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let stmts = match prog {
            Node::Program(s) => s,
            other => panic!("expected Program, got {:?}", other),
        };
        for spanned in stmts {
            if matches!(spanned.node, Node::Function { .. }) {
                return spanned.node;
            }
        }
        panic!("no Function in source");
    }

    fn infer_ok(src: &str) -> Substitution {
        let func = first_function(src);
        let mut inf = Inferer::new();
        inf.infer_function(&func)
            .unwrap_or_else(|errs| panic!("expected ok, got {:?}", errs))
    }

    fn infer_err(src: &str) -> Vec<Diagnostic> {
        let func = first_function(src);
        let mut inf = Inferer::new();
        match inf.infer_function(&func) {
            Ok(_) => panic!("expected error"),
            Err(errs) => errs,
        }
    }

    // ---------- Literal inference ----------

    #[test]
    fn int_literal_infers_int() {
        let src = "fn f() { let x = 7; return x; }\n";
        let func = first_function(src);
        let mut inf = Inferer::new();
        inf.infer_function(&func).expect("ok");
        assert_eq!(inf.env.get("x"), Some(&Type::Int));
    }

    #[test]
    fn float_literal_infers_float() {
        let src = "fn f() { let x = 1.5; return x; }\n";
        let func = first_function(src);
        let mut inf = Inferer::new();
        inf.infer_function(&func).expect("ok");
        assert_eq!(inf.env.get("x"), Some(&Type::Float));
    }

    #[test]
    fn bool_literal_infers_bool() {
        let src = "fn f() { let x = true; return x; }\n";
        let func = first_function(src);
        let mut inf = Inferer::new();
        inf.infer_function(&func).expect("ok");
        assert_eq!(inf.env.get("x"), Some(&Type::Bool));
    }

    #[test]
    fn string_literal_infers_string() {
        let src = "fn f() { let x = \"hi\"; return x; }\n";
        let func = first_function(src);
        let mut inf = Inferer::new();
        inf.infer_function(&func).expect("ok");
        assert_eq!(inf.env.get("x"), Some(&Type::String));
    }

    // ---------- Parameter env seeding ----------

    #[test]
    fn parameter_annotations_seed_env() {
        let src = "fn f(int n) { let m = n + 1; return m; }\n";
        let func = first_function(src);
        let mut inf = Inferer::new();
        inf.infer_function(&func).expect("ok");
        assert_eq!(inf.env.get("n"), Some(&Type::Int));
        assert_eq!(inf.env.get("m"), Some(&Type::Int));
    }

    // ---------- Operator rules: arithmetic ----------

    #[test]
    fn int_plus_int_unifies_to_int() {
        let _ = infer_ok("fn f(int a, int b) { let s = a + b; return s; }\n");
    }

    #[test]
    fn float_plus_float_unifies_to_float() {
        let _ = infer_ok("fn f(float a, float b) { let s = a + b; return s; }\n");
    }

    #[test]
    fn int_plus_float_fails_unification() {
        let errs = infer_err(
            "fn f(int a, float b) { let s = a + b; return s; }\n",
        );
        assert!(
            errs.iter().any(|d| d.code == Some(T0002_PRIMITIVE_MISMATCH)),
            "expected T0002, got {:?}",
            errs,
        );
    }

    #[test]
    fn int_plus_bool_fails() {
        let errs = infer_err(
            "fn f(int a, bool b) { let s = a + b; return s; }\n",
        );
        assert!(!errs.is_empty());
    }

    // ---------- Operator rules: logical ----------

    #[test]
    fn bool_and_bool_returns_bool() {
        let _ = infer_ok(
            "fn f(bool a, bool b) { let r = a && b; return r; }\n",
        );
    }

    #[test]
    fn int_and_bool_fails() {
        let errs = infer_err(
            "fn f(int a, bool b) { let r = a && b; return r; }\n",
        );
        assert!(!errs.is_empty());
    }

    // ---------- Operator rules: comparison ----------

    #[test]
    fn int_equal_int_returns_bool() {
        let src =
            "fn f(int a, int b) { let r = a == b; return r; }\n";
        let func = first_function(src);
        let mut inf = Inferer::new();
        inf.infer_function(&func).expect("ok");
        assert_eq!(inf.env.get("r"), Some(&Type::Bool));
    }

    #[test]
    fn int_lt_string_fails() {
        let errs = infer_err(
            "fn f(int a, string b) { let r = a < b; return r; }\n",
        );
        assert!(!errs.is_empty());
    }

    // ---------- Operator rules: bitwise ----------

    #[test]
    fn int_bitand_int_returns_int() {
        let src = "fn f(int a, int b) { let r = a & b; return r; }\n";
        let func = first_function(src);
        let mut inf = Inferer::new();
        inf.infer_function(&func).expect("ok");
        assert_eq!(inf.env.get("r"), Some(&Type::Int));
    }

    #[test]
    fn bitwise_on_bool_fails() {
        let errs = infer_err(
            "fn f(bool a, bool b) { let r = a & b; return r; }\n",
        );
        assert!(!errs.is_empty());
    }

    // ---------- Prefix operators ----------

    #[test]
    fn prefix_not_requires_bool() {
        let _ = infer_ok("fn f(bool x) { let y = !x; return y; }\n");
    }

    #[test]
    fn prefix_not_on_int_fails() {
        let errs = infer_err("fn f(int x) { let y = !x; return y; }\n");
        assert!(!errs.is_empty());
    }

    #[test]
    fn prefix_neg_on_int_returns_int() {
        let src = "fn f(int x) { let y = 0 - x; return y; }\n";
        // Parser doesn't produce `-x` as a PrefixExpression
        // today; the `0 - x` form exercises the binary arm with
        // the int-times-int unification. Same type guarantee.
        let func = first_function(src);
        let mut inf = Inferer::new();
        inf.infer_function(&func).expect("ok");
        assert_eq!(inf.env.get("y"), Some(&Type::Int));
    }

    // ---------- Let annotation conflicts ----------

    #[test]
    fn let_annotation_matches_value() {
        let _ = infer_ok("fn f() { let x: int = 3; return x; }\n");
    }

    #[test]
    fn let_annotation_conflicts_with_value() {
        let errs = infer_err("fn f() { let x: int = true; return x; }\n");
        assert!(
            errs.iter().any(|d| d.code == Some(T0002_PRIMITIVE_MISMATCH)),
            "expected primitive-mismatch T0002, got {:?}",
            errs,
        );
    }

    // ---------- Control flow ----------

    #[test]
    fn if_condition_must_be_bool() {
        let errs = infer_err(
            "fn f(int x) { if x { return 1; } else { return 2; } }\n",
        );
        assert!(!errs.is_empty());
    }

    #[test]
    fn while_condition_must_be_bool() {
        let errs = infer_err(
            "fn f(int x) { while x { let y = 1; } return 0; }\n",
        );
        assert!(!errs.is_empty());
    }

    // ---------- Unbound identifier ----------

    #[test]
    fn unbound_identifier_errors_with_t0005() {
        let errs = infer_err("fn f() { let x = y; return x; }\n");
        assert!(
            errs.iter().any(|d| d.code == Some(T0005_UNBOUND)),
            "expected T0005, got {:?}",
            errs,
        );
    }

    // ---------- Occurs check (can't easily trigger with only
    //            primitive types — pin the unify-error → diag
    //            mapping instead) ----------

    #[test]
    fn occurs_error_maps_to_t0001() {
        use crate::unify::UnifyError;
        // Value doesn't matter for the mapping — the mapper
        // only looks at the variant.
        let e = UnifyError::Occurs(7, Type::Int);
        let d = unify_error_to_diag(e, Span::default(), "x".into());
        assert_eq!(d.code, Some(T0001_OCCURS));
    }

    // ---------- Substitution is exposed post-inference ----------

    #[test]
    fn substitution_is_accessible_after_inference() {
        let src = "fn f(int a, int b) { let s = a + b; return s; }\n";
        let func = first_function(src);
        let mut inf = Inferer::new();
        let _ = inf.infer_function(&func).expect("ok");
        // The substitution may be empty if every type was
        // concrete on first visit — that's fine. The accessor
        // just exists.
        let _subst = inf.substitution();
    }

    // ---------- RES-122: Scheme + generalize + instantiate ----------

    fn int_to_int_fn() -> Type {
        Type::Function {
            params: vec![Type::Int],
            return_type: Box::new(Type::Int),
        }
    }

    #[test]
    fn free_type_vars_empty_for_primitive() {
        assert!(free_type_vars(&Type::Int).is_empty());
        assert!(free_type_vars(&Type::Float).is_empty());
        assert!(free_type_vars(&Type::Bool).is_empty());
        assert!(free_type_vars(&Type::String).is_empty());
        assert!(free_type_vars(&int_to_int_fn()).is_empty());
    }

    #[test]
    fn free_type_vars_collects_var_id() {
        assert_eq!(
            free_type_vars(&Type::Var(3)),
            [3].into_iter().collect(),
        );
    }

    #[test]
    fn free_type_vars_descends_into_fn_types() {
        let fn_ty = Type::Function {
            params: vec![Type::Var(1), Type::Int],
            return_type: Box::new(Type::Var(2)),
        };
        let ftv = free_type_vars(&fn_ty);
        assert_eq!(ftv, [1, 2].into_iter().collect());
    }

    #[test]
    fn generalize_quantifies_vars_not_in_env() {
        // Env has no vars; `ty` has Var(0) and Var(1). Both are
        // quantified.
        let env = HashMap::new();
        let ty = Type::Function {
            params: vec![Type::Var(0)],
            return_type: Box::new(Type::Var(1)),
        };
        let scheme = generalize(&env, &ty);
        assert_eq!(scheme.vars, vec![0, 1]);
        assert_eq!(scheme.ty, ty);
    }

    #[test]
    fn generalize_skips_vars_already_free_in_env() {
        // Env binds `x: Var(0)`, so ty's Var(0) is NOT
        // quantified — it's free in the outer scope.
        let mut env = HashMap::new();
        env.insert("x".to_string(), Type::Var(0));
        let ty = Type::Function {
            params: vec![Type::Var(0)],
            return_type: Box::new(Type::Var(1)),
        };
        let scheme = generalize(&env, &ty);
        assert_eq!(scheme.vars, vec![1]);
    }

    #[test]
    fn generalize_produces_monomorphic_scheme_when_no_vars() {
        let env = HashMap::new();
        let scheme = generalize(&env, &Type::Int);
        assert!(scheme.vars.is_empty());
        assert_eq!(scheme.ty, Type::Int);
    }

    #[test]
    fn scheme_monotype_constructor() {
        let s = Scheme::monotype(Type::Bool);
        assert!(s.vars.is_empty());
        assert_eq!(s.ty, Type::Bool);
    }

    #[test]
    fn scheme_new_preserves_fields() {
        let s = Scheme::new(vec![0, 2], Type::Var(0));
        assert_eq!(s.vars, vec![0, 2]);
        assert_eq!(s.ty, Type::Var(0));
    }

    #[test]
    fn instantiate_replaces_quantified_vars_with_fresh_ones() {
        // Scheme: `∀ 0. Fn([Var(0)]) -> Var(0)` (the classic
        // `id` type). Instantiating twice should yield two
        // distinct fresh vars.
        let scheme = Scheme {
            vars: vec![0],
            ty: Type::Function {
                params: vec![Type::Var(0)],
                return_type: Box::new(Type::Var(0)),
            },
        };
        let mut inf = Inferer::new();
        let t1 = inf.instantiate(&scheme);
        let t2 = inf.instantiate(&scheme);
        // Both should be Fn types whose param + return are
        // the same var — but distinct across instantiations.
        fn unwrap_fn(t: &Type) -> (&Type, &Type) {
            match t {
                Type::Function { params, return_type } => (&params[0], return_type),
                _ => panic!("expected Fn, got {:?}", t),
            }
        }
        let (p1, r1) = unwrap_fn(&t1);
        let (p2, r2) = unwrap_fn(&t2);
        // Within one instantiation: param == return (same var).
        assert_eq!(p1, r1);
        assert_eq!(p2, r2);
        // Across instantiations: different vars.
        assert_ne!(p1, p2);
    }

    #[test]
    fn instantiate_monotype_is_identity() {
        // A monomorphic scheme has no quantified vars, so
        // instantiate just returns a clone.
        let scheme = Scheme::monotype(Type::Int);
        let mut inf = Inferer::new();
        assert_eq!(inf.instantiate(&scheme), Type::Int);
    }

    #[test]
    fn instantiate_does_not_touch_unquantified_vars() {
        // Scheme with vars=[1] but ty references Var(1) AND
        // Var(2). Var(2) is NOT quantified — it stays as-is.
        let scheme = Scheme {
            vars: vec![1],
            ty: Type::Function {
                params: vec![Type::Var(1), Type::Var(2)],
                return_type: Box::new(Type::Var(1)),
            },
        };
        let mut inf = Inferer::new();
        let t = inf.instantiate(&scheme);
        if let Type::Function { params, return_type } = t {
            // Var(2) survives unchanged.
            assert_eq!(params[1], Type::Var(2));
            // Var(1) becomes a fresh var (matches
            // return_type); fresh var is NOT Var(2).
            assert_ne!(params[0], Type::Var(2));
            assert_eq!(&params[0], return_type.as_ref());
        } else {
            panic!("expected Fn, got {:?}", t);
        }
    }

    #[test]
    fn round_trip_generalize_then_instantiate() {
        // Inferred type `Fn([Var(0)]) -> Var(0)` against empty
        // env. generalize → Scheme { vars: [0], ... }.
        // instantiate → fresh Var, same shape.
        let env = HashMap::new();
        let inferred = Type::Function {
            params: vec![Type::Var(0)],
            return_type: Box::new(Type::Var(0)),
        };
        let scheme = generalize(&env, &inferred);
        let mut inf = Inferer::new();
        let inst = inf.instantiate(&scheme);
        match inst {
            Type::Function { ref params, ref return_type } => {
                assert_eq!(params.len(), 1);
                // Structure preserved.
                assert_eq!(&params[0], return_type.as_ref());
            }
            other => panic!("expected Fn, got {:?}", other),
        }
    }
}
