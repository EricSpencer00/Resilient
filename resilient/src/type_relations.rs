//! Shared type-relation helpers used by the typechecker, variance
//! analysis, and generic substitution code.
//!
//! The goal is to keep the semantic rules in one place so the compiler
//! has a single source of truth for:
//!
//! - compatibility checks between two concrete types
//! - conservative subtype checks for variance
//! - recursive type-parameter substitution
//! - inference-friendly return-type substitution at generic call sites

#![allow(dead_code)]

use crate::typechecker::Type;
use std::collections::HashMap;

pub(crate) fn is_pinned_int(t: &Type) -> bool {
    matches!(
        t,
        Type::Int8
            | Type::Int16
            | Type::Int32
            | Type::UInt8
            | Type::UInt16
            | Type::UInt32
            | Type::UInt64
    )
}

fn infer_common_type_inner(types: &[Type]) -> Type {
    let mut result: Option<&Type> = None;
    for t in types {
        if matches!(t, Type::Any) {
            continue;
        }
        match result {
            None => result = Some(t),
            Some(r) if r == t => {}
            _ => return Type::Any,
        }
    }
    result.cloned().unwrap_or(Type::Any)
}

pub(crate) fn infer_common_type(types: &[Type]) -> Type {
    infer_common_type_inner(types)
}

/// RES-3933 A-E3 follow-up (#4067): `true` when `name` is an
/// associated-type projection `X::Assoc` whose base `X` is one of the
/// callee's generic type parameters. Such a projection is *opaque* at
/// a generic call site — its concrete identity depends on which impl
/// binds it, context that only exists at monomorphization time — so
/// substitution maps it to `Type::Any` exactly like a bare `T`.
/// Without this, `Type::Struct("T::Item")` survives substitution
/// verbatim and can never structurally match any real argument or
/// binding, falsely rejecting every call to a fn with a
/// parameter-position projection. Well-formedness of the projection
/// itself (does some trait bound of `X` declare `Assoc`?) is
/// validated separately in `associated_types::check`.
fn is_type_param_projection(name: &str, type_params: &[String]) -> bool {
    name.split_once("::")
        .is_some_and(|(base, _)| type_params.iter().any(|p| p == base))
}

pub(crate) fn substitute_type_params(ty: &Type, type_params: &[String]) -> Type {
    match ty {
        Type::Struct(name)
            if type_params.iter().any(|p| p == name)
                || is_type_param_projection(name, type_params) =>
        {
            Type::Any
        }
        Type::Function {
            params,
            return_type,
        } => Type::Function {
            params: params
                .iter()
                .map(|p| substitute_type_params(p, type_params))
                .collect(),
            return_type: Box::new(substitute_type_params(return_type, type_params)),
        },
        Type::Tuple(elems) => Type::Tuple(
            elems
                .iter()
                .map(|e| substitute_type_params(e, type_params))
                .collect(),
        ),
        Type::AnonymousStruct(fields) => Type::AnonymousStruct(
            fields
                .iter()
                .map(|(name, ty)| (name.clone(), substitute_type_params(ty, type_params)))
                .collect(),
        ),
        Type::Option(inner) => Type::Option(Box::new(substitute_type_params(inner, type_params))),
        other => other.clone(),
    }
}

pub(crate) fn substitute_with_bindings(
    ty: &Type,
    type_params: &[String],
    bindings: &HashMap<&str, Type>,
) -> Type {
    match ty {
        Type::Struct(name) if type_params.iter().any(|p| p == name) => {
            bindings.get(name.as_str()).cloned().unwrap_or(Type::Any)
        }
        // A projection off a type parameter (`T::Item`) is opaque at
        // the call site even when `T` itself has a binding — resolving
        // it to the impl's concrete bound type needs the trait/impl
        // tables, which this pure helper doesn't have. `Any` is the
        // sound permissive fallback (see #4067).
        Type::Struct(name) if is_type_param_projection(name, type_params) => Type::Any,
        Type::Function {
            params,
            return_type,
        } => Type::Function {
            params: params
                .iter()
                .map(|p| substitute_with_bindings(p, type_params, bindings))
                .collect(),
            return_type: Box::new(substitute_with_bindings(return_type, type_params, bindings)),
        },
        Type::Tuple(elems) => Type::Tuple(
            elems
                .iter()
                .map(|e| substitute_with_bindings(e, type_params, bindings))
                .collect(),
        ),
        Type::AnonymousStruct(fields) => Type::AnonymousStruct(
            fields
                .iter()
                .map(|(name, ty)| {
                    (
                        name.clone(),
                        substitute_with_bindings(ty, type_params, bindings),
                    )
                })
                .collect(),
        ),
        Type::Option(inner) => Type::Option(Box::new(substitute_with_bindings(
            inner,
            type_params,
            bindings,
        ))),
        other => other.clone(),
    }
}

pub(crate) fn infer_generic_return_type(
    return_type: &Type,
    callee_type_params: &Option<Vec<String>>,
    tp_bindings: &HashMap<&str, Type>,
) -> Type {
    let tp = match callee_type_params {
        Some(tp) if !tp.is_empty() => tp,
        _ => return return_type.clone(),
    };
    substitute_with_bindings(return_type, tp, tp_bindings)
}

/// Conservative subtype relation used by the variance checker and any
/// future call-site relation logic.
///
/// The current lattice is intentionally small:
/// - exact equality always holds
/// - `Any` is the top type
/// - tuples are covariant element-wise
/// - `Option<T>` is covariant in `T`
/// - function types are contravariant in parameters and covariant in
///   their return type
/// - pinned integer widths do not form a subtype chain; they remain
///   equal-only until an explicit conversion or a declared widening
///   relation exists
pub(crate) fn is_subtype(sub: &Type, sup: &Type) -> bool {
    if sub == sup {
        return true;
    }
    if matches!(sup, Type::Any) {
        return true;
    }
    if matches!(sub, Type::Any) {
        return false;
    }
    match (sub, sup) {
        (Type::Option(a), Type::Option(b)) => is_subtype(a, b),
        (
            Type::Function {
                params: a_params,
                return_type: a_ret,
            },
            Type::Function {
                params: b_params,
                return_type: b_ret,
            },
        ) => {
            a_params.len() == b_params.len()
                && a_params
                    .iter()
                    .zip(b_params.iter())
                    .all(|(a, b)| is_subtype(b, a))
                && is_subtype(a_ret, b_ret)
        }
        (Type::Tuple(a_elems), Type::Tuple(b_elems)) => {
            a_elems.len() == b_elems.len()
                && a_elems
                    .iter()
                    .zip(b_elems.iter())
                    .all(|(a, b)| is_subtype(a, b))
        }
        _ => false,
    }
}

/// Compatibility is a slightly looser relation than subtype: it keeps
/// the existing numeric-literal and `Any` ergonomics used by the
/// typechecker.
pub(crate) fn compatible(a: &Type, b: &Type) -> bool {
    if a == b {
        return true;
    }
    if matches!(a, Type::Any) || matches!(b, Type::Any) {
        return true;
    }
    if let (Type::Option(inner_a), Type::Option(inner_b)) = (a, b) {
        return compatible(inner_a, inner_b);
    }
    if let (
        Type::Function {
            params: pa,
            return_type: ra,
        },
        Type::Function {
            params: pb,
            return_type: rb,
        },
    ) = (a, b)
    {
        return pa.len() == pb.len()
            && pa.iter().zip(pb.iter()).all(|(x, y)| compatible(x, y))
            && compatible(ra, rb);
    }
    if let (Type::Tuple(a_elems), Type::Tuple(b_elems)) = (a, b) {
        return a_elems.len() == b_elems.len()
            && a_elems
                .iter()
                .zip(b_elems.iter())
                .all(|(x, y)| compatible(x, y));
    }
    if *a == Type::Int && is_pinned_int(b) {
        return true;
    }
    if is_pinned_int(a) && *b == Type::Int {
        return true;
    }
    if (*a == Type::Float32 && *b == Type::Float) || (*a == Type::Float && *b == Type::Float32) {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtype_is_covariant_for_option_and_tuple() {
        let sup = Type::Option(Box::new(Type::Any));
        let sub = Type::Option(Box::new(Type::Int));
        assert!(is_subtype(&sub, &sup));

        let sup = Type::Tuple(vec![Type::Any, Type::Bool]);
        let sub = Type::Tuple(vec![Type::Int, Type::Bool]);
        assert!(is_subtype(&sub, &sup));
    }

    #[test]
    fn subtype_is_function_contravariant_in_params() {
        let sup = Type::Function {
            params: vec![Type::Int],
            return_type: Box::new(Type::Int),
        };
        let sub = Type::Function {
            params: vec![Type::Any],
            return_type: Box::new(Type::Int),
        };
        assert!(is_subtype(&sub, &sup));
    }

    #[test]
    fn compatibility_keeps_numeric_ergonomics() {
        assert!(compatible(&Type::Int, &Type::UInt16));
        assert!(compatible(&Type::Float, &Type::Float32));
    }

    #[test]
    fn recursive_type_parameter_substitution_works() {
        let ty = Type::Function {
            params: vec![Type::Struct("T".to_string())],
            return_type: Box::new(Type::Tuple(vec![
                Type::Struct("U".to_string()),
                Type::Option(Box::new(Type::Struct("T".to_string()))),
            ])),
        };
        let out = substitute_type_params(&ty, &["T".into(), "U".into()]);
        assert_eq!(
            out,
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Tuple(vec![
                    Type::Any,
                    Type::Option(Box::new(Type::Any)),
                ])),
            }
        );
    }

    #[test]
    fn recursive_substitution_preserves_anonymous_struct_shape() {
        let ty = Type::AnonymousStruct(vec![
            ("left".to_string(), Type::Struct("T".to_string())),
            (
                "right".to_string(),
                Type::Option(Box::new(Type::Struct("U".to_string()))),
            ),
        ]);
        let out = substitute_type_params(&ty, &["T".into(), "U".into()]);
        assert_eq!(
            out,
            Type::AnonymousStruct(vec![
                ("left".to_string(), Type::Any),
                ("right".to_string(), Type::Option(Box::new(Type::Any))),
            ])
        );
    }

    #[test]
    fn bound_substitution_preserves_anonymous_struct_shape() {
        let ty = Type::AnonymousStruct(vec![
            ("value".to_string(), Type::Struct("T".to_string())),
            (
                "nested".to_string(),
                Type::Tuple(vec![Type::Struct("U".to_string()), Type::Int]),
            ),
        ]);
        let mut bindings = HashMap::new();
        bindings.insert("T", Type::Bool);
        bindings.insert("U", Type::String);
        let out = substitute_with_bindings(&ty, &["T".into(), "U".into()], &bindings);
        assert_eq!(
            out,
            Type::AnonymousStruct(vec![
                ("value".to_string(), Type::Bool),
                (
                    "nested".to_string(),
                    Type::Tuple(vec![Type::String, Type::Int]),
                ),
            ])
        );
    }

    #[test]
    fn common_type_skips_any() {
        let got = infer_common_type(&[Type::Any, Type::Int, Type::Int]);
        assert_eq!(got, Type::Int);
    }
}

// =========================================================================
// RES-3814: Regression corpus for type_relations validation
// =========================================================================

#[test]
fn valid_pinned_int_int8() {
    assert!(is_pinned_int(&Type::Int8));
}

#[test]
fn valid_pinned_int_int16() {
    assert!(is_pinned_int(&Type::Int16));
}

#[test]
fn valid_pinned_int_int32() {
    assert!(is_pinned_int(&Type::Int32));
}

#[test]
fn valid_pinned_int_uint8() {
    assert!(is_pinned_int(&Type::UInt8));
}

#[test]
fn valid_pinned_int_uint16() {
    assert!(is_pinned_int(&Type::UInt16));
}

#[test]
fn valid_pinned_int_uint32() {
    assert!(is_pinned_int(&Type::UInt32));
}

#[test]
fn valid_pinned_int_uint64() {
    assert!(is_pinned_int(&Type::UInt64));
}

#[test]
fn malformed_pinned_int_float() {
    assert!(!is_pinned_int(&Type::Float));
}

#[test]
fn malformed_pinned_int_bool() {
    assert!(!is_pinned_int(&Type::Bool));
}

#[test]
fn malformed_pinned_int_string() {
    assert!(!is_pinned_int(&Type::String));
}

#[test]
fn malformed_pinned_int_any() {
    assert!(!is_pinned_int(&Type::Any));
}

#[test]
fn valid_common_type_all_same() {
    let got = infer_common_type(&[Type::Int, Type::Int, Type::Int]);
    assert_eq!(got, Type::Int);
}

#[test]
fn valid_common_type_mixed_with_any() {
    let got = infer_common_type(&[Type::Any, Type::String, Type::Any, Type::String]);
    assert_eq!(got, Type::String);
}

#[test]
fn valid_common_type_single_element() {
    let got = infer_common_type(&[Type::Bool]);
    assert_eq!(got, Type::Bool);
}

#[test]
fn malformed_common_type_conflicting() {
    let got = infer_common_type(&[Type::Int, Type::String, Type::Bool]);
    assert_eq!(got, Type::Any);
}

#[test]
fn malformed_common_type_empty_array() {
    let got = infer_common_type(&[]);
    assert_eq!(got, Type::Any);
}

#[test]
fn valid_substitute_type_params_basic() {
    let ty = Type::Struct("T".to_string());
    let out = substitute_type_params(&ty, &["T".into()]);
    assert_eq!(out, Type::Any);
}

#[test]
fn valid_substitute_type_params_none() {
    let ty = Type::Int;
    let out = substitute_type_params(&ty, &["T".into()]);
    assert_eq!(out, Type::Int);
}

#[test]
fn valid_substitute_bindings_basic() {
    let ty = Type::Struct("T".to_string());
    let mut bindings = HashMap::new();
    bindings.insert("T", Type::Bool);
    let out = substitute_with_bindings(&ty, &["T".into()], &bindings);
    assert_eq!(out, Type::Bool);
}

#[test]
fn malformed_substitute_unknown_binding() {
    let ty = Type::Struct("T".to_string());
    let bindings = HashMap::new();
    let out = substitute_with_bindings(&ty, &["T".into()], &bindings);
    assert_eq!(out, Type::Any);
}

#[test]
fn edge_case_nested_option_type() {
    let ty = Type::Option(Box::new(Type::Option(Box::new(Type::Int))));
    let out = substitute_type_params(&ty, &[]);
    assert_eq!(out, ty);
}

#[test]
fn edge_case_complex_tuple_substitution() {
    let ty = Type::Tuple(vec![
        Type::Struct("T".to_string()),
        Type::Int,
        Type::Struct("U".to_string()),
    ]);
    let out = substitute_type_params(&ty, &["T".into(), "U".into()]);
    assert_eq!(out, Type::Tuple(vec![Type::Any, Type::Int, Type::Any]));
}

#[test]
fn edge_case_function_type_substitution() {
    let ty = Type::Function {
        params: vec![Type::Struct("T".to_string()), Type::Int],
        return_type: Box::new(Type::Struct("T".to_string())),
    };
    let out = substitute_type_params(&ty, &["T".into()]);
    assert_eq!(
        out,
        Type::Function {
            params: vec![Type::Any, Type::Int],
            return_type: Box::new(Type::Any),
        }
    );
}
