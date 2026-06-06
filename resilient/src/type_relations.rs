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

pub(crate) fn substitute_type_params(ty: &Type, type_params: &[String]) -> Type {
    match ty {
        Type::Struct(name) if type_params.iter().any(|p| p == name) => Type::Any,
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
