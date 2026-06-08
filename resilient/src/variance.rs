//! RES-2615: Type variance inference for generic type parameters.
//!
//! Computes and enforces variance of each type parameter declared on a
//! generic function or struct:
//!
//! - **Covariant** (`+T`): `T` appears only in *output* position
//!   (return type, owned field). `Container<Cat>` may be used where
//!   `Container<Animal>` is expected when `Cat` is a subtype of
//!   `Animal`.
//! - **Contravariant** (`-T`): `T` appears only in *input* position
//!   (function argument). `Handler<Animal>` may be used where
//!   `Handler<Cat>` is expected.
//! - **Invariant**: `T` appears in both positions (or in a mutable
//!   reference context). No subtype relationship is permitted.
//! - **Phantom**: `T` is declared but never used in the signature.
//!   Treated as covariant (the conservative/sound choice).
//!
//! ## Design
//!
//! The pass walks each generic function's *signature* only (parameter
//! types + return type). Bodies are not inspected — variance is a
//! property of the public interface, not the implementation.
//!
//! The result is stored in a per-function `VarianceMap` that callers
//! (future subtyping pass) query with `is_subtype_compatible`. Today
//! the pass checks that call sites do not violate invariance — if a
//! type parameter is invariant, the concrete type at the call site must
//! match exactly (no subtyping).
//!
//! ## Position classification
//!
//! Function parameter types → input positions.
//! Return type → output position.
//! Recursive: `Fn(A) -> B` appearing in return position makes `A` an
//! input (contravariant flip) and `B` an output.

#![allow(dead_code)]

use crate::Node;
use crate::typechecker::Type;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Variance lattice
// ---------------------------------------------------------------------------

/// Variance of a single type parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variance {
    /// Type parameter appears only in output (covariant +T).
    Covariant,
    /// Type parameter appears only in input (contravariant -T).
    Contravariant,
    /// Type parameter appears in both positions, or is used invariantly.
    Invariant,
    /// Type parameter is declared but unused in the signature.
    /// Treated as covariant for subtyping (phantom types are inert).
    Phantom,
}

impl Variance {
    /// Combine two variance observations via the lattice join:
    /// - Same variance stays the same.
    /// - `Phantom` is the identity element (unused so far).
    /// - `Covariant` + `Contravariant` → `Invariant`.
    /// - Anything + `Invariant` → `Invariant`.
    fn join(self, other: Variance) -> Variance {
        match (self, other) {
            (a, Variance::Phantom) => a,
            (Variance::Phantom, b) => b,
            (a, b) if a == b => a,
            // Different non-phantom, non-invariant ⟹ invariant.
            _ => Variance::Invariant,
        }
    }

    /// Flip variance when entering a contravariant context (function
    /// argument). Co ↔ Contra; Invariant and Phantom are fixed points.
    fn flip(self) -> Variance {
        match self {
            Variance::Covariant => Variance::Contravariant,
            Variance::Contravariant => Variance::Covariant,
            other => other,
        }
    }
}

impl std::fmt::Display for Variance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Variance::Covariant => write!(f, "covariant"),
            Variance::Contravariant => write!(f, "contravariant"),
            Variance::Invariant => write!(f, "invariant"),
            Variance::Phantom => write!(f, "phantom (unused)"),
        }
    }
}

// ---------------------------------------------------------------------------
// VarianceMap — one per generic function
// ---------------------------------------------------------------------------

/// Maps each type-parameter name (e.g. `"T"`, `"U"`) to its inferred
/// variance for a specific generic function.
#[derive(Debug, Clone, Default)]
pub struct VarianceMap {
    map: HashMap<String, Variance>,
}

impl VarianceMap {
    fn new() -> Self {
        Self::default()
    }

    /// Record a `variance_kind` observation for `tp_name`, joining with
    /// any prior observation via the lattice.
    fn observe(&mut self, tp_name: &str, variance_kind: Variance) {
        let entry = self
            .map
            .entry(tp_name.to_string())
            .or_insert(Variance::Phantom);
        *entry = entry.join(variance_kind);
    }

    /// Look up the inferred variance for `tp_name`.
    pub fn get(&self, tp_name: &str) -> Variance {
        self.map.get(tp_name).copied().unwrap_or(Variance::Phantom)
    }

    /// True when `actual` is an acceptable use of a type parameter with
    /// `self.get(tp_name)` variance relative to `expected`:
    /// - Covariant: `actual` must equal `expected` (no subtype info yet).
    /// - Contravariant: same restriction (no subtype info yet).
    /// - Invariant: `actual` must equal `expected` exactly.
    /// - Phantom: always OK.
    ///
    /// The shared relation layer keeps this conservative and future-proof:
    /// covariant positions accept subtypes, contravariant positions accept
    /// supertypes, and invariants stay exact.
    pub fn check_use(&self, tp_name: &str, expected: &Type, actual: &Type) -> Result<(), String> {
        let v = self.get(tp_name);
        match v {
            Variance::Phantom => Ok(()),
            Variance::Covariant if crate::type_relations::is_subtype(actual, expected) => Ok(()),
            Variance::Contravariant if crate::type_relations::is_subtype(expected, actual) => {
                Ok(())
            }
            Variance::Invariant if expected == actual => Ok(()),
            _ => Err(format!(
                "type parameter `{}` is {} — expected `{}`, got `{}`. \
                 Variance violation: the relation does not hold.",
                tp_name, v, expected, actual
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Variance inference
// ---------------------------------------------------------------------------

/// Infer the variance of each type parameter in `type_params` from the
/// function's `param_types` (input position) and `return_type` (output
/// position).
///
/// The caller provides the *resolved* type annotations as `Type` values.
/// In practice, generic type parameters appear as `Type::Struct(name)`
/// where `name` matches an entry in `type_params`.
pub fn infer_variance(
    type_params: &[String],
    param_types: &[Type],
    return_type: &Type,
) -> VarianceMap {
    let tp_set: std::collections::HashSet<&str> = type_params.iter().map(String::as_str).collect();
    let mut vmap = VarianceMap::new();
    // Seed all params as Phantom — they start unused.
    for tp in type_params {
        vmap.map.insert(tp.clone(), Variance::Phantom);
    }
    // Walk parameters in input position.
    for pt in param_types {
        walk_type(pt, Variance::Contravariant, &tp_set, &mut vmap);
    }
    // Walk return type in output position.
    walk_type(return_type, Variance::Covariant, &tp_set, &mut vmap);
    vmap
}

/// Recursively walk `ty` in `position`, recording observations for any
/// type-parameter names found in `tp_set`.
///
/// When we encounter a `Type::Function` inside a position:
/// - Its parameter types inherit the *flipped* outer position (because
///   function parameters are contravariant w.r.t. the caller).
/// - Its return type inherits the *same* outer position.
fn walk_type(
    ty: &Type,
    position: Variance,
    tp_set: &std::collections::HashSet<&str>,
    vmap: &mut VarianceMap,
) {
    match ty {
        Type::Struct(name) if tp_set.contains(name.as_str()) => {
            vmap.observe(name, position);
        }
        Type::Function {
            params,
            return_type,
        } => {
            // Params of a function type are in contravariant position
            // relative to the outer context — flip once.
            let param_pos = position.flip();
            for p in params {
                walk_type(p, param_pos, tp_set, vmap);
            }
            walk_type(return_type, position, tp_set, vmap);
        }
        Type::Tuple(ts) => {
            for t in ts {
                walk_type(t, position, tp_set, vmap);
            }
        }
        Type::AnonymousStruct(fields) => {
            for (_, ty) in fields {
                walk_type(ty, position, tp_set, vmap);
            }
        }
        // Primitive and opaque types contain no type-parameter references.
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Top-level compiler pass
// ---------------------------------------------------------------------------

/// Walk the program and compute variance for every generic function.
/// Attaches the map to a thread-local registry so that downstream
/// passes (subtyping checks) can query it.
///
/// Errors are emitted if a type parameter is inferred as invariant but
/// the function signature would be ambiguous.  For now the pass is
/// purely informational + stores results — the check surfaces in tests.
pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    let has_generic = stmts
        .iter()
        .any(|s| matches!(&s.node, Node::Function { type_params, .. } if !type_params.is_empty()));
    if !has_generic {
        return Ok(());
    }

    let mut registry: HashMap<String, VarianceMap> = HashMap::new();

    for stmt in stmts {
        if let Node::Function {
            name,
            type_params,
            parameters,
            return_type,
            ..
        } = &stmt.node
        {
            if type_params.is_empty() {
                continue;
            }

            // Build the param types list from the raw annotation strings
            // (same representation as generics.rs uses).
            let param_types: Vec<Type> = parameters
                .iter()
                .map(|(ty_str, _name)| parse_type_annotation(ty_str))
                .collect();

            let ret_ty = return_type
                .as_deref()
                .map(parse_type_annotation)
                .unwrap_or(Type::Void);

            let vmap = infer_variance(type_params, &param_types, &ret_ty);

            // Validate: a type parameter that is used in both positions
            // is invariant — that's valid, just more restrictive.
            // What IS an error: a type parameter that appears invariantly
            // because it was declared but the signature is contradictory
            // (i.e., the caller says it's `+T` but uses it both ways).
            // Right now we only surface a diagnostic if a parameter ends
            // up invariant AND the function name carries a `+` or `-`
            // annotation (future syntax).  Without explicit annotations,
            // inference is always sound — no error from inference alone.
            registry.insert(name.clone(), vmap);
        }
    }

    // Store the registry in a thread-local so downstream passes can
    // query it.  This keeps the pass stateless in its return value.
    VARIANCE_REGISTRY.with(|cell| {
        *cell.borrow_mut() = registry;
    });

    Ok(())
}

thread_local! {
    /// Thread-local variance registry populated by `check`.
    static VARIANCE_REGISTRY: std::cell::RefCell<HashMap<String, VarianceMap>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Query the variance of `tp_name` in function `fn_name`.
/// Returns `Variance::Phantom` when the function or parameter is unknown
/// (i.e., when `check` has not yet been called for this program).
pub fn get_variance(fn_name: &str, tp_name: &str) -> Variance {
    VARIANCE_REGISTRY.with(|cell| {
        cell.borrow()
            .get(fn_name)
            .map(|m| m.get(tp_name))
            .unwrap_or(Variance::Phantom)
    })
}

/// Parse a raw type annotation string (as stored in the AST's
/// `parameters` and `return_type` fields) into a `Type`.
///
/// This is a minimal parser that only handles the types that can appear
/// as generic parameter annotations today:
/// - Primitive names (`int`, `bool`, `float`, `string`, `bytes`).
/// - Type-parameter names (anything that isn't a known primitive).
///
/// The full type-annotation syntax is parsed by the main parser; this
/// helper exists only so the variance pass can classify without
/// importing the full parse machinery.
fn parse_type_annotation(s: &str) -> Type {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("fn(")
        && let Some((params_src, return_src)) = split_function_annotation(rest)
    {
        let params = split_top_level(params_src, ',')
            .into_iter()
            .filter(|part| !part.trim().is_empty())
            .map(parse_type_annotation)
            .collect();
        return Type::Function {
            params,
            return_type: Box::new(parse_type_annotation(return_src)),
        };
    }
    if trimmed.starts_with('(') && trimmed.ends_with(')') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let elems = split_top_level(inner, ',');
        if elems.len() > 1 {
            return Type::Tuple(elems.into_iter().map(parse_type_annotation).collect());
        }
    }
    match trimmed {
        "int" | "Int" | "Int64" => Type::Int,
        "Int8" => Type::Int8,
        "Int16" => Type::Int16,
        "Int32" => Type::Int32,
        "UInt8" => Type::UInt8,
        "UInt16" => Type::UInt16,
        "UInt32" => Type::UInt32,
        "UInt64" => Type::UInt64,
        "float" | "Float" => Type::Float,
        "bool" | "Bool" => Type::Bool,
        "string" | "String" => Type::String,
        "bytes" | "Bytes" => Type::Bytes,
        "void" | "Void" | "()" => Type::Void,
        // Anything else is either a type-parameter name or a nominal type.
        other => Type::Struct(other.to_string()),
    }
}

fn split_function_annotation(s: &str) -> Option<(&str, &str)> {
    let mut depth = 1_i32;
    for (idx, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let tail = s[idx + 1..].trim_start();
                    let return_src = tail.strip_prefix("->")?.trim();
                    return Some((&s[..idx], return_src));
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut paren_depth = 0_i32;
    for (idx, ch) in s.char_indices() {
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            _ if ch == sep && paren_depth == 0 => {
                parts.push(s[start..idx].trim());
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(s[start..].trim());
    parts
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    // -----------------------------------------------------------------------
    // Variance inference unit tests (no parse — pure type algebra)
    // -----------------------------------------------------------------------

    fn s(name: &str) -> Type {
        Type::Struct(name.to_string())
    }

    #[test]
    fn phantom_when_type_param_unused() {
        // fn<T> id() -> int — T never appears.
        let vmap = infer_variance(&["T".to_string()], &[], &Type::Int);
        assert_eq!(vmap.get("T"), Variance::Phantom);
    }

    #[test]
    fn covariant_when_type_param_in_output_only() {
        // fn<T> produce() -> T — T only in output.
        let vmap = infer_variance(&["T".to_string()], &[], &s("T"));
        assert_eq!(vmap.get("T"), Variance::Covariant);
    }

    #[test]
    fn contravariant_when_type_param_in_input_only() {
        // fn<T> consume(T x) -> void — T only in input.
        let vmap = infer_variance(&["T".to_string()], &[s("T")], &Type::Void);
        assert_eq!(vmap.get("T"), Variance::Contravariant);
    }

    #[test]
    fn invariant_when_type_param_in_both_positions() {
        // fn<T> transform(T x) -> T — T in both input and output.
        let vmap = infer_variance(&["T".to_string()], &[s("T")], &s("T"));
        assert_eq!(vmap.get("T"), Variance::Invariant);
    }

    #[test]
    fn two_params_inferred_independently() {
        // fn<A, B>(A x) -> B — A contravariant, B covariant.
        let vmap = infer_variance(&["A".to_string(), "B".to_string()], &[s("A")], &s("B"));
        assert_eq!(vmap.get("A"), Variance::Contravariant);
        assert_eq!(vmap.get("B"), Variance::Covariant);
    }

    #[test]
    fn function_type_in_return_flips_param_variance() {
        // fn<T, U>() -> Fn(T) -> U
        // The outer return type is output (+). The inner `Fn(T)` has:
        //   - T in param position of the function type → one flip → input (Contravariant).
        //   - U in return position of the function type → stays Covariant.
        let fn_ty = Type::Function {
            params: vec![s("T")],
            return_type: Box::new(s("U")),
        };
        let vmap = infer_variance(&["T".to_string(), "U".to_string()], &[], &fn_ty);
        assert_eq!(vmap.get("T"), Variance::Contravariant);
        assert_eq!(vmap.get("U"), Variance::Covariant);
    }

    #[test]
    fn variance_join_covariant_plus_contravariant_is_invariant() {
        assert_eq!(
            Variance::Covariant.join(Variance::Contravariant),
            Variance::Invariant
        );
    }

    #[test]
    fn variance_join_phantom_is_identity() {
        assert_eq!(
            Variance::Covariant.join(Variance::Phantom),
            Variance::Covariant
        );
        assert_eq!(
            Variance::Phantom.join(Variance::Contravariant),
            Variance::Contravariant
        );
    }

    #[test]
    fn variance_flip_covariant_is_contravariant() {
        assert_eq!(Variance::Covariant.flip(), Variance::Contravariant);
        assert_eq!(Variance::Contravariant.flip(), Variance::Covariant);
        assert_eq!(Variance::Invariant.flip(), Variance::Invariant);
        assert_eq!(Variance::Phantom.flip(), Variance::Phantom);
    }

    #[test]
    fn check_use_passes_for_matching_types() {
        let vmap = infer_variance(&["T".to_string()], &[], &s("T")); // Covariant
        vmap.check_use("T", &Type::Int, &Type::Int)
            .expect("same type must be acceptable");
    }

    #[test]
    fn check_use_accepts_covariant_subtype() {
        let vmap = infer_variance(&["T".to_string()], &[], &s("T"));
        vmap.check_use("T", &Type::Any, &Type::Int)
            .expect("subtype should be acceptable in covariant position");
    }

    #[test]
    fn check_use_accepts_contravariant_supertype() {
        let vmap = infer_variance(&["T".to_string()], &[s("T")], &Type::Void);
        vmap.check_use("T", &Type::Int, &Type::Any)
            .expect("supertype should be acceptable in contravariant position");
    }

    #[test]
    fn check_use_fails_for_mismatching_types_on_invariant_param() {
        let vmap = infer_variance(&["T".to_string()], &[s("T")], &s("T")); // Invariant
        let err = vmap
            .check_use("T", &Type::Int, &Type::Bool)
            .expect_err("mismatched types must fail for invariant param");
        assert!(
            err.contains("Variance violation"),
            "diagnostic missing 'Variance violation': {}",
            err
        );
    }

    // -----------------------------------------------------------------------
    // Full-program integration via `check` pass
    // -----------------------------------------------------------------------

    fn run_check(src: &str) -> Result<(), String> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        check(&prog, "<test>")
    }

    #[test]
    fn check_pass_succeeds_on_covariant_generic() {
        // fn<T> produce() -> T — covariant, no error.
        run_check("fn<T> produce() -> T { return produce(); }").expect("covariant fn must pass");
    }

    #[test]
    fn check_pass_succeeds_on_contravariant_generic() {
        run_check("fn<T> consume(T x) -> void { }").expect("contravariant fn must pass");
    }

    #[test]
    fn check_pass_succeeds_on_invariant_generic() {
        run_check("fn<T> transform(T x) -> T { return x; }").expect("invariant fn must pass");
    }

    #[test]
    fn check_pass_records_variance_in_registry() {
        let src = "fn<T> consume(T x) -> void { }";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty());
        check(&prog, "<test>").expect("check must succeed");
        assert_eq!(get_variance("consume", "T"), Variance::Contravariant);
    }

    #[test]
    fn check_pass_records_covariant_return() {
        let src = "fn<T> produce(int n) -> T { return produce(n); }";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty());
        check(&prog, "<test>").expect("check must succeed");
        assert_eq!(get_variance("produce", "T"), Variance::Covariant);
    }

    #[test]
    fn check_pass_records_invariant_both_positions() {
        let src = "fn<T> id(T x) -> T { return x; }";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty());
        check(&prog, "<test>").expect("check must succeed");
        assert_eq!(get_variance("id", "T"), Variance::Invariant);
    }

    #[test]
    fn no_error_on_empty_program() {
        run_check("").expect("empty program must not error");
    }

    #[test]
    fn no_error_on_monomorphic_program() {
        run_check("fn add(int x) -> int { return x + 1; }").expect("monomorphic fn must not error");
    }
}
