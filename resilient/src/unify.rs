#![allow(dead_code)]
// RES-121: RES-120 (the inference walker that is the only non-test
// caller of this module) is still OPEN pending rewrite, so every
// non-test exported item is unused on `main` today. Suppressed
// module-wide; RES-120 removes this attribute when it lands.

//! RES-121: Hindley-Milner style first-order unification with an
//! occurs check.
//!
//! Split from the inference walker (RES-120, currently blocked) so
//! this module can evolve and be tested independently. The data
//! model:
//!
//! - `Substitution` maps type-variable ids (the `u32` inside
//!   `Type::Var(id)`) to the concrete (or other-variable-chained)
//!   `Type` they have been unified with.
//! - `apply` walks a `Type` and replaces any `Var(id)` whose binding
//!   is recorded in the substitution. Idempotent: applying twice
//!   equals applying once (asserted in the unit tests).
//! - `unify(a, b)` grows `self` so that `apply(a) == apply(b)`, or
//!   returns `UnifyError` describing why that was impossible.
//! - `compose(other)` returns a substitution equivalent to "apply
//!   `other` first, then `self`" — the order that keeps unify walks
//!   correct (see doc-comment on `compose`).
//!
//! The occurs check prevents binding `Var(v)` to a type that
//! transitively references `Var(v)` — the classic protection against
//! infinite types like `t = List<t>`. It is present from day one,
//! even though the primitive-only surface can't produce such a type;
//! RES-124 needs it.

use std::collections::HashMap;

use crate::typechecker::Type;

/// Reasons `unify` may fail. The callers wrap these into higher-
/// level diagnostics (RES-119, when it lands).
#[derive(Debug, Clone, PartialEq)]
pub enum UnifyError {
    /// Two concrete types can't be made equal (e.g. `Int` vs `Bool`).
    Mismatch(Type, Type),
    /// Attempted to bind `Var(v)` to a type whose `apply` image still
    /// contains `Var(v)`.
    Occurs(u32, Type),
    /// Function types with differing parameter arity.
    ArityMismatch(usize, usize),
}

impl std::fmt::Display for UnifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnifyError::Mismatch(a, b) => write!(f, "cannot unify `{}` with `{}`", a, b),
            UnifyError::Occurs(v, t) => {
                write!(f, "occurs check: `?t{}` appears inside `{}`", v, t)
            }
            UnifyError::ArityMismatch(a, b) => {
                write!(f, "function arity mismatch: {} vs {}", a, b)
            }
        }
    }
}

impl std::error::Error for UnifyError {}

/// Mapping from type-variable ids to the `Type` they've been bound
/// to. An empty substitution is the identity.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Substitution {
    inner: HashMap<u32, Type>,
}

impl Substitution {
    /// A fresh (identity) substitution.
    pub fn new() -> Self {
        Self { inner: HashMap::new() }
    }

    /// The underlying map — exposed as `&` so tests can assert on
    /// bindings without granting mutation access.
    pub fn as_map(&self) -> &HashMap<u32, Type> {
        &self.inner
    }

    /// Walk `ty` and replace every `Var(id)` whose binding this
    /// substitution carries. Applied recursively through `Var` chains
    /// so `{0 -> Var(1), 1 -> Int}` with input `Var(0)` yields `Int`
    /// in one pass.
    ///
    /// Idempotent: `apply(apply(ty)) == apply(ty)`.
    pub fn apply(&self, ty: &Type) -> Type {
        match ty {
            Type::Var(id) => {
                // Follow the chain; break on cycles defensively
                // (shouldn't happen — `unify`'s occurs check prevents
                // cycles — but a bug elsewhere shouldn't deadlock).
                let mut seen: Vec<u32> = Vec::new();
                let mut cur_id = *id;
                loop {
                    if seen.contains(&cur_id) {
                        // Broken invariant — return the original var
                        // rather than loop forever.
                        return Type::Var(cur_id);
                    }
                    seen.push(cur_id);
                    match self.inner.get(&cur_id) {
                        None => return Type::Var(cur_id),
                        Some(Type::Var(next_id)) => {
                            cur_id = *next_id;
                        }
                        Some(other) => return self.apply(other),
                    }
                }
            }
            Type::Function { params, return_type } => Type::Function {
                params: params.iter().map(|p| self.apply(p)).collect(),
                return_type: Box::new(self.apply(return_type)),
            },
            // Primitive and opaque variants have no sub-types.
            Type::Int
            | Type::Float
            | Type::String
            | Type::Bytes
            | Type::Bool
            | Type::Array
            | Type::Result
            | Type::Struct(_)
            | Type::Void
            | Type::Any => ty.clone(),
        }
    }

    /// In-place unify `a` with `b`, growing `self` with any new
    /// bindings required. On error, `self` is left in whatever
    /// partial state the unification reached — callers that care
    /// about rollback should clone before calling.
    pub fn unify(&mut self, a: &Type, b: &Type) -> Result<(), UnifyError> {
        let a = self.apply(a);
        let b = self.apply(b);
        match (a, b) {
            // Reflexive primitive equality.
            (Type::Int, Type::Int)
            | (Type::Float, Type::Float)
            | (Type::String, Type::String)
            | (Type::Bool, Type::Bool)
            | (Type::Array, Type::Array)
            | (Type::Result, Type::Result)
            | (Type::Void, Type::Void) => Ok(()),
            // Nominal structs unify iff their names match.
            (Type::Struct(n1), Type::Struct(n2)) if n1 == n2 => Ok(()),
            // `Any` is the inference-ready "unknown" from the old
            // typechecker; accept it against anything for now so the
            // prototype can coexist with the RES-053 nominal checker.
            (Type::Any, _) | (_, Type::Any) => Ok(()),
            // Var-on-either-side: bind (with occurs check).
            (Type::Var(a), Type::Var(b)) if a == b => Ok(()),
            (Type::Var(v), other) => self.bind(v, other),
            (other, Type::Var(v)) => self.bind(v, other),
            // Function: unify arities and then per-component.
            (
                Type::Function { params: p1, return_type: r1 },
                Type::Function { params: p2, return_type: r2 },
            ) => {
                if p1.len() != p2.len() {
                    return Err(UnifyError::ArityMismatch(p1.len(), p2.len()));
                }
                for (l, r) in p1.iter().zip(p2.iter()) {
                    self.unify(l, r)?;
                }
                self.unify(&r1, &r2)
            }
            (a, b) => Err(UnifyError::Mismatch(a, b)),
        }
    }

    /// `compose(other)` returns a new substitution equivalent to
    /// "apply `other` first, then `self`". Concretely: for every
    /// binding `(v -> t)` in `other`, the result carries
    /// `(v -> self.apply(t))`; bindings only in `self` are copied
    /// through unchanged.
    ///
    /// This ordering is what keeps unify walks correct: when the
    /// walker unifies two sub-terms independently and wants to merge
    /// their substitutions, it needs the newer walk's bindings
    /// (`self`) to see through the older walk's (`other`). Getting
    /// the order wrong drops the newer substitutions on the floor.
    pub fn compose(&self, other: &Substitution) -> Substitution {
        let mut out = Substitution::new();
        for (&v, t) in &other.inner {
            out.inner.insert(v, self.apply(t));
        }
        for (&v, t) in &self.inner {
            // `other.inner` bindings already got composed above —
            // copy `self`'s bindings through unchanged for vars
            // `other` didn't touch.
            out.inner.entry(v).or_insert_with(|| t.clone());
        }
        out
    }

    /// Bind `Var(v)` to `ty`, running the occurs check first.
    /// `ty` is already the `apply` image (so the check is meaningful).
    fn bind(&mut self, v: u32, ty: Type) -> Result<(), UnifyError> {
        if let Type::Var(v2) = ty
            && v == v2
        {
            return Ok(());
        }
        if self.occurs(v, &ty) {
            return Err(UnifyError::Occurs(v, ty));
        }
        self.inner.insert(v, ty);
        Ok(())
    }

    /// True if `Var(v)` appears anywhere inside `ty` (through the
    /// current substitution).
    fn occurs(&self, v: u32, ty: &Type) -> bool {
        match ty {
            Type::Var(id) => {
                if *id == v {
                    return true;
                }
                // Walk through chained bindings.
                if let Some(bound) = self.inner.get(id) {
                    return self.occurs(v, bound);
                }
                false
            }
            Type::Function { params, return_type } => {
                params.iter().any(|p| self.occurs(v, p)) || self.occurs(v, return_type)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prim_unifies_with_itself() {
        let mut s = Substitution::new();
        assert!(s.unify(&Type::Int, &Type::Int).is_ok());
        assert!(s.unify(&Type::Bool, &Type::Bool).is_ok());
        assert!(s.unify(&Type::Float, &Type::Float).is_ok());
        assert!(s.unify(&Type::String, &Type::String).is_ok());
        assert!(s.as_map().is_empty(), "no bindings produced for prim-prim");
    }

    #[test]
    fn prim_mismatch_errors_with_the_two_types() {
        let mut s = Substitution::new();
        let err = s.unify(&Type::Int, &Type::Bool).unwrap_err();
        assert_eq!(err, UnifyError::Mismatch(Type::Int, Type::Bool));
    }

    #[test]
    fn var_unifies_to_prim() {
        let mut s = Substitution::new();
        s.unify(&Type::Var(0), &Type::Int).unwrap();
        assert_eq!(s.apply(&Type::Var(0)), Type::Int);
    }

    #[test]
    fn var_unifies_to_var_and_then_to_prim() {
        let mut s = Substitution::new();
        s.unify(&Type::Var(0), &Type::Var(1)).unwrap();
        s.unify(&Type::Var(1), &Type::Bool).unwrap();
        assert_eq!(s.apply(&Type::Var(0)), Type::Bool);
        assert_eq!(s.apply(&Type::Var(1)), Type::Bool);
    }

    #[test]
    fn var_equal_to_itself_is_noop() {
        let mut s = Substitution::new();
        s.unify(&Type::Var(7), &Type::Var(7)).unwrap();
        assert!(s.as_map().is_empty());
    }

    #[test]
    fn occurs_check_catches_direct_self_reference() {
        // Construct `Fn(Var(0)) -> Int`, then try to unify `Var(0)`
        // with it. Occurs-check must reject.
        let recursive = Type::Function {
            params: vec![Type::Var(0)],
            return_type: Box::new(Type::Int),
        };
        let mut s = Substitution::new();
        let err = s.unify(&Type::Var(0), &recursive).unwrap_err();
        assert_eq!(err, UnifyError::Occurs(0, recursive));
    }

    #[test]
    fn occurs_check_catches_indirect_self_reference_via_chain() {
        // 0 -> 1, then try to unify 1 with Fn(Var(0)) -> Int.
        // Occurs check walks the chain and refuses.
        let mut s = Substitution::new();
        s.unify(&Type::Var(0), &Type::Var(1)).unwrap();
        let recursive = Type::Function {
            params: vec![Type::Var(0)],
            return_type: Box::new(Type::Int),
        };
        let err = s.unify(&Type::Var(1), &recursive).unwrap_err();
        // The bound form of the failing var is what the error reports.
        match err {
            UnifyError::Occurs(v, _) => assert_eq!(v, 1),
            other => panic!("expected Occurs, got {:?}", other),
        }
    }

    #[test]
    fn apply_is_idempotent_on_three_variable_chain() {
        // 0 -> 1, 1 -> 2, 2 -> Int. Applying to Var(0) should give
        // Int, and applying again should still give Int.
        let mut s = Substitution::new();
        s.unify(&Type::Var(0), &Type::Var(1)).unwrap();
        s.unify(&Type::Var(1), &Type::Var(2)).unwrap();
        s.unify(&Type::Var(2), &Type::Int).unwrap();
        let once = s.apply(&Type::Var(0));
        let twice = s.apply(&once);
        assert_eq!(once, Type::Int);
        assert_eq!(twice, once);
    }

    #[test]
    fn apply_recurses_into_function_types() {
        let mut s = Substitution::new();
        s.unify(&Type::Var(0), &Type::Int).unwrap();
        s.unify(&Type::Var(1), &Type::Bool).unwrap();
        let fun = Type::Function {
            params: vec![Type::Var(0)],
            return_type: Box::new(Type::Var(1)),
        };
        let applied = s.apply(&fun);
        assert_eq!(
            applied,
            Type::Function {
                params: vec![Type::Int],
                return_type: Box::new(Type::Bool),
            }
        );
    }

    #[test]
    fn unify_function_types_elementwise() {
        let a = Type::Function {
            params: vec![Type::Var(0), Type::Bool],
            return_type: Box::new(Type::Var(1)),
        };
        let b = Type::Function {
            params: vec![Type::Int, Type::Bool],
            return_type: Box::new(Type::Float),
        };
        let mut s = Substitution::new();
        s.unify(&a, &b).unwrap();
        assert_eq!(s.apply(&Type::Var(0)), Type::Int);
        assert_eq!(s.apply(&Type::Var(1)), Type::Float);
    }

    #[test]
    fn unify_function_arity_mismatch_errors() {
        let a = Type::Function {
            params: vec![Type::Var(0)],
            return_type: Box::new(Type::Int),
        };
        let b = Type::Function {
            params: vec![Type::Int, Type::Bool],
            return_type: Box::new(Type::Int),
        };
        let mut s = Substitution::new();
        let err = s.unify(&a, &b).unwrap_err();
        assert_eq!(err, UnifyError::ArityMismatch(1, 2));
    }

    #[test]
    fn compose_apply_other_first_then_self() {
        // other: 0 -> Var(1)
        // self:  1 -> Int
        // composed: 0 -> Int (because we apply `other` first to get
        // Var(1), then self to get Int), 1 -> Int (self's binding).
        let mut other = Substitution::new();
        other.unify(&Type::Var(0), &Type::Var(1)).unwrap();
        let mut me = Substitution::new();
        me.unify(&Type::Var(1), &Type::Int).unwrap();
        let composed = me.compose(&other);
        assert_eq!(composed.apply(&Type::Var(0)), Type::Int);
        assert_eq!(composed.apply(&Type::Var(1)), Type::Int);
    }

    #[test]
    fn compose_preserves_self_bindings_for_unrelated_vars() {
        let mut other = Substitution::new();
        other.unify(&Type::Var(0), &Type::Int).unwrap();
        let mut me = Substitution::new();
        me.unify(&Type::Var(9), &Type::Bool).unwrap();
        let composed = me.compose(&other);
        assert_eq!(composed.apply(&Type::Var(0)), Type::Int);
        assert_eq!(composed.apply(&Type::Var(9)), Type::Bool);
    }

    #[test]
    fn any_accepts_anything_for_back_compat_with_res053() {
        let mut s = Substitution::new();
        s.unify(&Type::Any, &Type::Int).unwrap();
        s.unify(&Type::Bool, &Type::Any).unwrap();
    }

    #[test]
    fn struct_mismatch_errors() {
        let mut s = Substitution::new();
        let err = s
            .unify(&Type::Struct("A".into()), &Type::Struct("B".into()))
            .unwrap_err();
        match err {
            UnifyError::Mismatch(a, b) => {
                assert_eq!(a, Type::Struct("A".into()));
                assert_eq!(b, Type::Struct("B".into()));
            }
            other => panic!("expected Mismatch, got {:?}", other),
        }
    }
}
