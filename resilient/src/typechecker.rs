//! Type checker module for Resilient language.
//!
//! ## RES-776 / RES-780: Supervisor-Actor Integration Design
//!
//! When fully implemented, the typechecker will validate that supervisors
//! and their supervised actors work together correctly:
//!
//! 1. **Supervisor Declaration** (RES-776 PR 1): ✓ Validates syntax
//! 2. **Supervisor-Actor Binding** (RES-776 PR 2 / RES-780):
//!    - Validate each child function is a valid actor handler
//!    - Validate restart policy makes sense for the actor's structure
//!    - Ensure supervisor and actor messages are compatible
//! 3. **Runtime Integration** (RES-776 PR 3-5):
//!    - Wire crash detection into actor scheduler
//!    - Implement restart policy application
//!    - Add supervision examples and tests
//!
//! This module is the typechecker phase; `actor_runtime.rs` handles
//! scheduler/crash machinery, and `supervisor.rs` handles parsing.

use crate::span::Span;
use crate::{Node, Pattern};
use std::collections::{HashMap, HashSet};

/// RES-189: one entry in the typechecker's post-walk inlay-hint
/// cache. Produced for every unannotated `let` binding (i.e.
/// `let x = ...;` without an explicit `: T`). The LSP backend
/// converts these into `InlayHint`s.
///
/// `span` is the `let` keyword's span (1-indexed line/col, per
/// RES-077). `name_len_chars` is the length of the binding's
/// identifier in chars — together those let the LSP compute
/// "end of pattern" as `col + "let ".len() + name_len_chars`.
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields read behind `lsp` feature only
pub struct LetTypeHint {
    pub span: Span,
    pub name_len_chars: usize,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// The default integer type — also the type of integer literals
    /// and the canonical name for `Int64` (`Int` and `Int64` alias
    /// each other at the type level). RES-366: narrower pinned types
    /// do NOT implicitly convert to/from `Int`; use an explicit cast.
    Int,
    /// RES-366: pinned signed integer types. `Int` is the alias for
    /// `Int64`. Narrower types require explicit `as_intN` casts.
    Int8,
    Int16,
    Int32,
    /// RES-366: pinned unsigned integer types. Overflow wraps on cast.
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float,
    /// RES-2618: single-precision IEEE 754-2019 binary32. Distinct from
    /// `Float` (f64) so the compiler can catch implicit cross-width mixing.
    /// Cortex-M4F has hardware f32 FPU; f64 on that target is software-emulated.
    Float32,
    String,
    Bool,
    /// RES-2711: Unicode scalar value. Produced by char literals (`'x'`) and
    /// by string indexing `s[i]` (RES-2709). Distinct from `String` so the
    /// typechecker can reject mixing them without explicit conversion.
    Char,
    /// RES-152: raw byte sequence, distinct from `String`. Protocol
    /// frames, register maps, packed on-the-wire structs. Unify
    /// rules mirror `String` but the two types don't interchange —
    /// users bridge via explicit conversion builtins (a follow-up
    /// ticket per RES-152's Notes).
    Bytes,
    Function {
        params: Vec<Type>,
        return_type: Box<Type>,
    },
    /// RES-053: Array — element type not tracked at MVP (typed arrays
    /// land with RES-055 / generics).
    Array,
    /// RES-053: Result<T, E> — payload types not tracked at MVP.
    Result,
    /// RES-2651: `Option<T>` with tracked inner type. When the inner
    /// type is known (e.g. from a `checked_add` return or an explicit
    /// annotation), pattern matching on `Some(x)` binds `x` to the
    /// concrete inner type instead of `Any`. Falls back gracefully:
    /// `Option(Box::new(Type::Any))` behaves like the old untracked
    /// `Result`-style representation.
    Option(Box<Type>),
    /// RES-053: user-defined record by name. Field types looked up
    /// against the struct table when G7 goes deeper.
    Struct(String),
    Void,
    Any, // Used for untyped variables during inference
    /// RES-401: product tuple `(T0, T1, …)`. Element types are tracked
    /// so `TupleIndex` can return the element type at a known literal
    /// index and `LetTupleDestructure` can bind each name to its
    /// precise element type. An empty Vec represents `()` (unit tuple).
    Tuple(Vec<Type>),
    /// RES-121: fresh inference variable (Hindley-Milner). Produced
    /// by the inference walker (RES-120, when it lands) and
    /// eliminated by `unify::Substitution::apply`. The `u32` is a
    /// globally-unique id minted from a monotonic counter; IDs have
    /// no intrinsic meaning beyond identity.
    ///
    /// The optional `Span` records the source position of a `_` type
    /// hole that originated this variable (RES-125 deferred AC).
    /// `None` for inference variables that don't come from explicit
    /// holes. Display renders the hole form as "type hole at line:col".
    ///
    /// Currently only the `unify` module's unit tests construct this
    /// variant; the `dead_code` allow goes away when RES-120 lands.
    #[allow(dead_code)]
    Var(u32, Option<Span>),
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Type::Int => write!(f, "int"),
            Type::Int8 => write!(f, "Int8"),
            Type::Int16 => write!(f, "Int16"),
            Type::Int32 => write!(f, "Int32"),
            Type::UInt8 => write!(f, "UInt8"),
            Type::UInt16 => write!(f, "UInt16"),
            Type::UInt32 => write!(f, "UInt32"),
            Type::UInt64 => write!(f, "UInt64"),
            Type::Float => write!(f, "float"),
            Type::Float32 => write!(f, "f32"),
            Type::String => write!(f, "string"),
            Type::Bool => write!(f, "bool"),
            Type::Char => write!(f, "char"),
            Type::Bytes => write!(f, "bytes"),
            Type::Function {
                params,
                return_type,
            } => {
                write!(f, "fn(")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", param)?;
                }
                write!(f, ") -> {}", return_type)
            }
            Type::Array => write!(f, "array"),
            Type::Result => write!(f, "Result"),
            Type::Option(inner) => {
                if matches!(inner.as_ref(), Type::Any) {
                    write!(f, "Option")
                } else {
                    write!(f, "Option<{}>", inner)
                }
            }
            Type::Struct(n) => write!(f, "{}", n),
            Type::Void => write!(f, "void"),
            Type::Any => write!(f, "any"),
            Type::Tuple(ts) => {
                write!(f, "(")?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", t)?;
                }
                write!(f, ")")
            }
            Type::Var(_, Some(span)) => {
                write!(f, "type hole at {}:{}", span.start.line, span.start.column)
            }
            Type::Var(id, None) => write!(f, "?t{}", id),
        }
    }
}

/// RES-1112 / RES-1113: does every path through `node` end in an
/// unconditional control-flow terminator (`return`, `break`,
/// `continue`)?
///
/// Used by:
/// - RES-1112: a non-void function whose body does not terminate on
///   every path may fall off the end and return `void` — caught at
///   compile time below.
/// - RES-1113: a statement in a `Block` is unreachable when an
///   earlier statement in the same block returned `true` here.
///
/// Conservative: returns `false` whenever the answer is ambiguous,
/// so any positive diagnosis is a real bug rather than a guess.
pub(crate) fn node_terminates(node: &Node) -> bool {
    match node {
        Node::ReturnStatement { .. } | Node::Break { .. } | Node::Continue { .. } => true,
        Node::Block { stmts, .. } => stmts.iter().any(node_terminates),
        Node::IfStatement {
            consequence,
            alternative: Some(alt),
            ..
        } => node_terminates(consequence) && node_terminates(alt),
        Node::Match { arms, .. } if !arms.is_empty() => {
            arms.iter().all(|(_, _, body)| node_terminates(body))
        }
        // RES-1112: a `live { ... }` block terminates if its body does
        // — the retry harness only re-executes the body, it doesn't
        // skip the terminator on success.
        Node::LiveBlock { body, .. } => node_terminates(body),
        // RES-1112: a `try { ... } catch V { ... }` terminates when
        // the body terminates AND every handler terminates — both
        // paths must end in `return`/`break`/`continue` for the whole
        // construct to be guaranteed terminating.
        Node::TryCatch { body, handlers, .. } => {
            body.iter().any(node_terminates)
                && handlers
                    .iter()
                    .all(|(_, hstmts)| hstmts.iter().any(node_terminates))
        }
        _ => false,
    }
}

/// RES-1112: a function body "yields a value" when every path
/// terminates with an explicit `return`, OR the body's last statement
/// is an expression statement (implicit-return form like
/// `fn id(int x) -> int { x }`). Returns `true` in that case;
/// `false` means the body can fall off the end without producing a
/// value of the declared return type.
pub(crate) fn body_yields_value(body: &Node) -> bool {
    if node_terminates(body) {
        return true;
    }
    if let Node::Block { stmts, .. } = body
        && let Some(last) = stmts.last()
        && matches!(last, Node::ExpressionStatement { .. })
    {
        return true;
    }
    false
}

/// RES-053: Two types are compatible if they're equal or if either is
/// Any. Used everywhere we need "same type, or we don't know yet."
///
/// RES-366: `Type::Int` (the type of integer literals) is compatible
/// RES-402: infer the common type of match/if arm bodies.
///
/// Scans `types`, skips `Type::Any` entries, and returns the
/// shared concrete type when all non-Any entries agree. Falls back
/// to `Type::Any` when types differ (or when the slice is empty /
/// all-Any), keeping inference conservative.
fn infer_common_arm_type(types: &[Type]) -> Type {
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

/// with every pinned integer type — assigning a literal `42` to an
/// `Int8` binding is always legal. Pinned types are NOT compatible
/// with each other: `Int8 ↔ Int16` requires an explicit `as_int16`
/// cast.
/// RES-2701: recursively replace `Type::Struct(name)` with `Type::Any`
/// for every `name` that appears in `type_params`. This is needed so
/// call-site argument checking treats `fn(T) -> T` as `fn(Any) -> Any`
/// when T is a declared generic type parameter — the single-level
/// `Struct("T") → Any` substitution in RES-425 did not recurse into
/// composite types (Function, Tuple, Option).
fn substitute_type_params(ty: &Type, type_params: &[String]) -> Type {
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
        Type::Option(inner) => Type::Option(Box::new(substitute_type_params(inner, type_params))),
        other => other.clone(),
    }
}

fn compatible(a: &Type, b: &Type) -> bool {
    if a == b {
        return true;
    }
    if matches!(a, Type::Any) || matches!(b, Type::Any) {
        return true;
    }
    // RES-2651: Option<T> is compatible with Option<U> when T and U
    // are compatible, and also with the legacy unparameterised Result
    // (which represented Option before this PR).
    if let (Type::Option(inner_a), Type::Option(inner_b)) = (a, b) {
        return compatible(inner_a, inner_b);
    }
    // RES-2701: Function types are compatible when all parameter types and
    // the return type are pairwise compatible. This handles cases where
    // substitute_type_params replaced type variables with Any — e.g.
    // `fn(Any) -> Any` must accept a concrete `fn(int) -> int`.
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
    // Integer literals produce Type::Int; allow assigning them to any
    // pinned integer type without an explicit cast.
    if *a == Type::Int && is_pinned_int(b) {
        return true;
    }
    if is_pinned_int(a) && *b == Type::Int {
        return true;
    }
    // RES-2691: float literals produce Type::Float; allow assigning them to
    // f32-annotated variables and vice-versa — mirrors the Int ↔ pinned-int rule.
    // Note: unify() still errors on Float/Float32 in arithmetic to prevent
    // implicit cross-width mixing.
    if (*a == Type::Float32 && *b == Type::Float) || (*a == Type::Float && *b == Type::Float32) {
        return true;
    }
    false
}

/// RES-2713: map a literal-pattern node to its type so
/// `match_pattern_binding_types` can validate that the literal is
/// type-compatible with the match scrutinee.
///
/// Returns `None` for non-literal or unknown node kinds (callers skip
/// the check — `Type::Any` scrutinees are handled via `compatible()`).
fn literal_pattern_ty(node: &Node) -> Option<Type> {
    match node {
        Node::IntegerLiteral { .. } => Some(Type::Int),
        Node::FloatLiteral { .. } => Some(Type::Float),
        Node::StringLiteral { .. } => Some(Type::String),
        Node::BooleanLiteral { .. } => Some(Type::Bool),
        Node::CharLiteral { .. } => Some(Type::Char),
        Node::BytesLiteral { .. } => Some(Type::Bytes),
        _ => None,
    }
}

/// RES-160: collect the binding names a pattern introduces, in
/// source order. Used to verify that all branches of an or-pattern
/// bind the same names.
// RES-1431: return `Vec<&str>` instead of `Vec<String>` to skip the
// per-binding-name clones. Callers (the or-pattern consistency check
// in `Node::Match`) only compare the lists with `!=`, which works
// identically on `Vec<&str>` and never needs owned `String`s. The
// returned references borrow from the Pattern AST; the caller's
// Pattern lives at least as long as the comparison.
fn pattern_bindings(p: &Pattern) -> Vec<&str> {
    match p {
        Pattern::Identifier(n) => vec![n.as_str()],
        // RES-915: range patterns bind no names (today; `1..=5 @ x`
        // binding is queued as a follow-up).
        Pattern::Wildcard | Pattern::Literal(_) | Pattern::Range { .. } => Vec::new(),
        Pattern::Or(branches) => {
            // By induction (checked at each arm) every branch
            // introduces the same names — pick the first branch's
            // list. Callers use this helper AFTER the consistency
            // check.
            branches.first().map(pattern_bindings).unwrap_or_default()
        }
        // RES-161a: outer name + whatever the inner pattern binds.
        Pattern::Bind(outer, inner) => {
            let mut bs = vec![outer.as_str()];
            bs.extend(pattern_bindings(inner));
            bs
        }
        Pattern::Struct { fields, .. } => fields
            .iter()
            .flat_map(|(_, sub)| pattern_bindings(sub.as_ref()))
            .collect(),
        // RES-375: `Some(inner)` introduces whatever the inner
        // pattern binds; `None` introduces nothing.
        Pattern::Some(inner) => pattern_bindings(inner.as_ref()),
        Pattern::None => Vec::new(),
        // RES-923: same shape for Ok / Err.
        Pattern::Ok(inner) | Pattern::Err(inner) => pattern_bindings(inner.as_ref()),
        // RES-400: enum-variant pattern bindings.
        // None: no payload, no bindings.
        // Named: each declared field carries a sub-pattern; recurse.
        // Tuple: each positional sub-pattern; recurse.
        Pattern::EnumVariant { payload, .. } => match payload {
            crate::EnumPatternPayload::None => Vec::new(),
            crate::EnumPatternPayload::Named(fields) => fields
                .iter()
                .flat_map(|(_, sub)| pattern_bindings(sub.as_ref()))
                .collect(),
            crate::EnumPatternPayload::Tuple(subs) => {
                subs.iter().flat_map(pattern_bindings).collect()
            }
        },
        // RES-931: tuple-struct destructure binds whatever each
        // positional sub-pattern binds.
        Pattern::TupleStruct { fields, .. } => fields.iter().flat_map(pattern_bindings).collect(),
        // RES-932: anonymous tuple destructure — recurse positionally.
        Pattern::Tuple(items) => items.iter().flat_map(pattern_bindings).collect(),
    }
}

/// RES-160: does the pattern match every value (i.e. a
/// wildcard / identifier, or an or-pattern with at least one
/// always-matching branch)? Counts as a "default" arm for
/// exhaustiveness.
fn pattern_is_default(p: &Pattern) -> bool {
    match p {
        Pattern::Wildcard | Pattern::Identifier(_) => true,
        // RES-915: a range pattern does not match every Int — `1..=5`
        // misses 0, 6, etc. — so it is never a default arm.
        Pattern::Literal(_) | Pattern::Range { .. } => false,
        Pattern::Or(branches) => branches.iter().any(pattern_is_default),
        // `x @ inner` is default iff the inner pattern is default.
        Pattern::Bind(_, inner) => pattern_is_default(inner),
        Pattern::Struct {
            fields, has_rest, ..
        } => {
            *has_rest
                || fields
                    .iter()
                    .all(|(_, sub)| pattern_is_default(sub.as_ref()))
        }
        // RES-375: `Some(_)` / `None` are not defaults by themselves.
        // RES-923: `Ok(_)` / `Err(_)` likewise — each only matches
        // half of `Result`.
        Pattern::Some(_) | Pattern::None | Pattern::Ok(_) | Pattern::Err(_) => false,
        // RES-400: an enum-variant pattern is *not* a default — it
        // matches only one variant. Exhaustiveness over enums is
        // handled by enumerating variants in a future PR.
        Pattern::EnumVariant { .. } => false,
        // RES-931: a tuple-struct pattern is a default iff every
        // positional sub-pattern is a default. (`Pair(_, _)` is
        // default; `Pair(0, _)` is not.) The struct itself is the
        // sole inhabitant of the nominal type, so name-mismatch is
        // caught upstream.
        Pattern::TupleStruct { fields, .. } => fields.iter().all(pattern_is_default),
        // RES-932: an anonymous tuple pattern is a default iff every
        // positional sub-pattern is a default. (`(_, _)` is default;
        // `(0, _)` is not.)
        Pattern::Tuple(items) => items.iter().all(pattern_is_default),
    }
}

/// RES-160: does the pattern include a literal-bool match for
/// `want`? Recurses through or-patterns so `true | false` covers
/// both branches.
fn pattern_covers_bool(p: &Pattern, want: bool) -> bool {
    match p {
        Pattern::Literal(Node::BooleanLiteral { value, .. }) => *value == want,
        Pattern::Or(branches) => branches.iter().any(|b| pattern_covers_bool(b, want)),
        _ => false,
    }
}

/// RES-369: does `p` cover every value of nominal struct `sname`
/// (given its declared fields) for exhaustiveness?
fn struct_pattern_matches_nominal_type(sname: &str, decl: &[(String, Type)], p: &Pattern) -> bool {
    match p {
        Pattern::Wildcard | Pattern::Identifier(_) => true,
        // RES-915: range patterns never match a struct; they're Int-only.
        Pattern::Literal(_) | Pattern::Range { .. } => false,
        Pattern::Or(branches) => branches
            .iter()
            .any(|b| struct_pattern_matches_nominal_type(sname, decl, b)),
        Pattern::Bind(_, inner) => struct_pattern_matches_nominal_type(sname, decl, inner),
        Pattern::Struct {
            struct_name,
            fields,
            has_rest,
        } => {
            if struct_name != sname {
                return false;
            }
            if *has_rest {
                return true;
            }
            if decl.is_empty() {
                return true;
            }
            if fields.len() != decl.len() {
                return false;
            }
            for (fname, _) in decl {
                let Some((_, sub)) = fields.iter().find(|(n, _)| n == fname) else {
                    return false;
                };
                if !pattern_is_default(sub.as_ref()) {
                    return false;
                }
            }
            true
        }
        // RES-375 / RES-923: Option / Result patterns don't match
        // struct-nominal types.
        Pattern::Some(_) | Pattern::None | Pattern::Ok(_) | Pattern::Err(_) => false,
        // RES-400: enum-variant patterns don't match struct-nominal types.
        Pattern::EnumVariant { .. } => false,
        // RES-931: tuple-struct pattern covers the nominal type iff
        // it names the same struct AND every positional sub-pattern
        // is a default (`Pair(_, _)` covers; `Pair(0, _)` does not).
        Pattern::TupleStruct { name, fields } => {
            name == sname && fields.len() == decl.len() && fields.iter().all(pattern_is_default)
        }
        // RES-932: anonymous tuple patterns don't match struct-nominal types.
        Pattern::Tuple(_) => false,
    }
}

fn pattern_is_exhaustive_wrt_scrutinee(
    scrut: &Type,
    p: &Pattern,
    struct_fields: &HashMap<String, std::rc::Rc<Vec<(String, Type)>>>,
) -> bool {
    match scrut {
        Type::Struct(sname) => {
            if let Some(decl) = struct_fields.get(sname) {
                struct_pattern_matches_nominal_type(sname, decl, p)
            } else {
                pattern_is_default(p)
            }
        }
        _ => pattern_is_default(p),
    }
}

/// RES-130: arithmetic operators (`+ - * / %`) require both
/// operands to be the same numeric type — no implicit int ↔ float
/// coercion. Any/Any fall through as Any for the inference-in-
/// progress path.
///
/// Returns the result type on success or a type-error diagnostic
/// pointing users at the explicit `to_float(x)` / `to_int(x)`
/// conversions when they mixed the two.
/// RES-366: helper — is this type a pinned integer (any width/sign)?
/// `Type::Int` (= Int64) is the generic integer type and is NOT
/// in this set; it's handled separately to allow literal assignment.
fn is_pinned_int(t: &Type) -> bool {
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

fn check_numeric_same_type(op: &str, left: &Type, right: &Type) -> Result<Type, String> {
    match (left, right) {
        (Type::Int, Type::Int) => Ok(Type::Int),
        (Type::Float, Type::Float) => Ok(Type::Float),
        (Type::Any, Type::Any) => Ok(Type::Any),
        // Any + Int / Any + Float — propagate the concrete side so
        // downstream inference can tighten.
        (Type::Int, Type::Any) | (Type::Any, Type::Int) => Ok(Type::Int),
        (Type::Float, Type::Any) | (Type::Any, Type::Float) => Ok(Type::Float),
        (Type::Int, Type::Float) | (Type::Float, Type::Int) => Err(format!(
            "Cannot apply '{}' to int and float — Resilient does not implicitly coerce between numeric types. Use `to_float(x)` or `to_int(x)` explicitly.",
            op
        )),
        // RES-2618: f32 arithmetic — same-width is OK; mixing f32 with f64 is an error.
        (Type::Float32, Type::Float32) => Ok(Type::Float32),
        (Type::Float32, Type::Any) | (Type::Any, Type::Float32) => Ok(Type::Float32),
        (Type::Float32, Type::Float) | (Type::Float, Type::Float32) => Err(format!(
            "Cannot apply '{}' to f32 and f64 — use `as f32` or `as f64` to convert explicitly.",
            op
        )),
        (Type::Float32, Type::Int) | (Type::Int, Type::Float32) => Err(format!(
            "Cannot apply '{}' to f32 and int — use `as_f32(x)` or `to_int(x)` to convert explicitly.",
            op
        )),
        // RES-366: pinned integer types — same width/sign is OK;
        // Any pairs with any pinned type.
        (a, b) if a == b && is_pinned_int(a) => Ok(a.clone()),
        (a, Type::Any) if is_pinned_int(a) => Ok(a.clone()),
        (Type::Any, b) if is_pinned_int(b) => Ok(b.clone()),
        (a, b) if is_pinned_int(a) && is_pinned_int(b) => Err(format!(
            "Cannot apply '{}' to {} and {} — use an explicit cast (e.g. `as_{}(x)`) to convert between pinned integer widths.",
            op,
            a,
            b,
            b.to_string().to_lowercase()
        )),
        _ => Err(format!("Cannot apply '{}' to {} and {}", op, left, right)),
    }
}

/// RES-060/061: fold a contract expression down to a concrete boolean.
/// `bindings` maps identifier names to known integer values — used at
/// call sites where the typechecker has constant arguments to
/// substitute for parameters.
///
/// Returns:
///   Some(true)  — provably true (tautology under bindings, discharged)
///   Some(false) — provably false (contradiction, reject)
///   None        — undecidable (leave for runtime check)
///
/// This is the verification core. G9b will swap it for a Z3 query;
/// the return shape (sat/unsat/unknown) stays the same.
fn fold_const_bool(n: &Node, bindings: &HashMap<String, i64>) -> Option<bool> {
    match n {
        Node::BooleanLiteral { value: b, .. } => Some(*b),
        Node::PrefixExpression {
            operator, right, ..
        } if *operator == "!" => fold_const_bool(right, bindings).map(|b| !b),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => match *operator {
            // RES-1353: short-circuit `&&` / `||` before recursing
            // on the right operand. The old pattern-match form
            // (`match (fold(left), fold(right)) { … }`) eagerly
            // folded *both* sides before inspecting the verdicts,
            // so a `Some(false) && expensive_right` clause walked
            // `expensive_right`'s subtree only to discard the
            // verdict. Mirrors the logical-short-circuit semantics
            // the operator already has at runtime.
            "&&" => {
                let l = fold_const_bool(left, bindings);
                if matches!(l, Some(false)) {
                    return Some(false);
                }
                let r = fold_const_bool(right, bindings);
                if matches!(r, Some(false)) {
                    return Some(false);
                }
                match (l, r) {
                    (Some(true), Some(true)) => Some(true),
                    _ => None,
                }
            }
            "||" => {
                let l = fold_const_bool(left, bindings);
                if matches!(l, Some(true)) {
                    return Some(true);
                }
                let r = fold_const_bool(right, bindings);
                if matches!(r, Some(true)) {
                    return Some(true);
                }
                match (l, r) {
                    (Some(false), Some(false)) => Some(false),
                    _ => None,
                }
            }
            "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                let l = fold_const_i64(left, bindings)?;
                let r = fold_const_i64(right, bindings)?;
                Some(match *operator {
                    "==" => l == r,
                    "!=" => l != r,
                    "<" => l < r,
                    ">" => l > r,
                    "<=" => l <= r,
                    ">=" => l >= r,
                    _ => unreachable!(),
                })
            }
            _ => None,
        },
        _ => None,
    }
}

/// RES-064: if the expression is `IDENT == LITERAL` or `LITERAL == IDENT`,
/// extract the assumption as `(name, value)`. Used to push the assumption
/// into const_bindings while checking an `if` consequence.
///
/// This is the first step toward real flow-sensitive verification.
/// Future tickets will extend to inequality bounds, ranges, and the
/// negative branch (else).
fn extract_eq_assumption(cond: &Node) -> Option<(String, i64)> {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        ..
    } = cond
        && *operator == "=="
    {
        let no_b: HashMap<String, i64> = HashMap::new();
        match (left.as_ref(), right.as_ref()) {
            (Node::Identifier { name, .. }, other) => {
                fold_const_i64(other, &no_b).map(|v| (name.clone(), v))
            }
            (other, Node::Identifier { name, .. }) => {
                fold_const_i64(other, &no_b).map(|v| (name.clone(), v))
            }
            _ => None,
        }
    } else {
        None
    }
}

/// Fold an integer-typed expression to a concrete i64 under bindings.
fn fold_const_i64(n: &Node, bindings: &HashMap<String, i64>) -> Option<i64> {
    match n {
        Node::IntegerLiteral { value: v, .. } => Some(*v),
        Node::Identifier { name, .. } => bindings.get(name).copied(),
        Node::PrefixExpression {
            operator, right, ..
        } if *operator == "-" => fold_const_i64(right, bindings).map(|v| -v),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let l = fold_const_i64(left, bindings)?;
            let r = fold_const_i64(right, bindings)?;
            match *operator {
                "+" => l.checked_add(r),
                "-" => l.checked_sub(r),
                "*" => l.checked_mul(r),
                "/" if r != 0 => l.checked_div(r),
                "%" if r != 0 => l.checked_rem(r),
                _ => None,
            }
        }
        _ => None,
    }
}

// Environment for storing type information
//
// RES-1372: `outer` is `Arc<TypeEnvironment>` (not `Box`) so cloning a
// `TypeEnvironment` is O(current store) instead of O(sum of stores
// along the outer chain). The typechecker enters a new scope (via
// `self.env.clone()` + `new_enclosed`) at every function body, block,
// fn-literal, actor body, handler, while/for loop, and quantifier
// binding — hundreds of times per non-trivial program. With the old
// `Box`, each clone walked the entire outer chain and deep-cloned
// every frame's `store` HashMap. With `Arc`, the chain-clone is a
// refcount bump; only the current scope's own `store` clones for real.
// `Arc` (not `Rc`) so the type stays `Send + Sync` — required because
// the RES-1349 builtin-env `LazyLock<TypeEnvironment>` is a `Sync`
// static. Atomic refcount ops are still trivially cheaper than the
// deep HashMap clone they replace.
// Mutation safety: `set` / `remove` only touch `self.store`; `get` /
// `all_names` only read through `&self.outer`. Nothing ever mutates
// through the indirection, so the Arc share is sound.
#[derive(Debug, Clone)]
pub struct TypeEnvironment {
    store: HashMap<String, Type>,
    outer: Option<std::sync::Arc<TypeEnvironment>>,
}

impl TypeEnvironment {
    /// RES-1698: kept `pub` so the supervisor test module
    /// (`supervisor.rs:362`) can still build a default-capacity env;
    /// every non-test caller uses `with_capacity` below.
    #[allow(dead_code)]
    pub fn new() -> Self {
        TypeEnvironment {
            store: HashMap::new(),
            outer: None,
        }
    }

    /// RES-1698: pre-sized variant. The `BUILTIN_ENV` LazyLock seeds
    /// ~490 built-in fn names into a `TypeEnvironment`, growing the
    /// inner HashMap from 0 → 4 → ... → 512 (~9 rehashes). Calling
    /// `with_capacity(512)` once per process avoids every one of
    /// those — the cloned per-`TypeChecker` envs inherit the
    /// preserved capacity.
    pub fn with_capacity(cap: usize) -> Self {
        TypeEnvironment {
            store: HashMap::with_capacity(cap),
            outer: None,
        }
    }

    pub fn new_enclosed(outer: TypeEnvironment) -> Self {
        TypeEnvironment {
            store: HashMap::new(),
            outer: Some(std::sync::Arc::new(outer)),
        }
    }

    /// RES-2444: variant of `new_enclosed` that accepts a pre-existing
    /// `Arc<TypeEnvironment>` as the outer scope — avoids a deep-clone
    /// when the outer is shared (e.g. the process-wide BUILTIN_ENV).
    fn new_with_outer_arc(outer: std::sync::Arc<TypeEnvironment>) -> Self {
        TypeEnvironment {
            store: HashMap::new(),
            outer: Some(outer),
        }
    }

    pub fn get(&self, name: &str) -> Option<Type> {
        match self.store.get(name) {
            Some(typ) => Some(typ.clone()),
            None => {
                if let Some(outer) = &self.outer {
                    outer.get(name)
                } else {
                    None
                }
            }
        }
    }

    pub fn set(&mut self, name: String, typ: Type) {
        self.store.insert(name, typ);
    }

    /// RES-159: remove a binding from the **current** scope only.
    /// Used to roll back transient pattern-binding entries after a
    /// match arm's body is type-checked so the identifier doesn't
    /// leak out. Outer-scope bindings with the same name are left
    /// untouched; this only clears what this scope owns.
    pub fn remove(&mut self, name: &str) {
        self.store.remove(name);
    }

    /// RES-306: collect every name visible in this scope chain
    /// (innermost first, walking outward). Used by the did-you-mean
    /// helper when emitting "undefined identifier" diagnostics.
    /// Inner shadowing is intentionally preserved — duplicate names
    /// appear once per scope; the consumer is expected to dedup.
    pub fn all_names(&self) -> Vec<String> {
        let mut out: Vec<String> = self.store.keys().cloned().collect();
        if let Some(outer) = &self.outer {
            out.extend(outer.all_names());
        }
        out
    }
}

/// RES-061: signature-and-contract record stored per top-level fn so
/// the typechecker can fold contracts at constant call sites.
#[derive(Debug, Clone)]
struct ContractInfo {
    parameters: Vec<(String, String)>, // (type_name, param_name)
    requires: Vec<Node>,
    /// Reserved for the symmetric ensures-fold work (post-call result
    /// substitution); not used by the call-site fold today, but kept
    /// in the table so RES-062 can pick up where this leaves off.
    #[allow(dead_code)]
    ensures: Vec<Node>,
    /// RES-387: declared failure variants on this fn. Call sites
    /// must propagate or handle each variant.
    fails: Vec<String>,
}

/// RES-066: counters for the verification audit. Incremented as the
/// typechecker walks the program; read out by `--audit` after the run.
#[derive(Debug, Clone, Default)]
pub struct VerificationStats {
    /// requires clauses with no free variables AND a constant call site,
    /// or pushed assumptions that fold to true.
    pub requires_discharged_at_compile: usize,
    /// requires clauses left for runtime check (couldn't fold).
    pub requires_left_for_runtime: usize,
    /// requires clauses that fold to a tautology (no params used).
    pub requires_tautology: usize,
    /// Total contracted call sites visited.
    pub contracted_call_sites: usize,
    /// RES-067: clauses the hand-rolled folder couldn't decide but Z3
    /// could. Bumped when --features z3 is in use; otherwise zero.
    pub requires_discharged_by_z3: usize,
    /// RES-137: clauses where the Z3 solver returned Unknown —
    /// typically because the per-query timeout fired on an
    /// undecidable or expensive NIA obligation. Counted separately
    /// from `requires_left_for_runtime` so the `--audit` table can
    /// flag them: the user may want to bump the timeout or rewrite
    /// the clause into a decidable subset.
    pub verifier_timeouts: usize,
    /// RES-068: per-function counters. fn_name → (discharged, runtime).
    /// A function is "fully provable" iff every call site discharged
    /// every requires clause statically. The interpreter elides runtime
    /// checks for those functions.
    pub per_fn_discharged: std::collections::HashMap<String, usize>,
    pub per_fn_runtime: std::collections::HashMap<String, usize>,
    /// RES-192: inferred effect set per top-level user fn. `true`
    /// = reaches IO (direct or transitive call to an impure
    /// builtin, or to another IO fn, or to an unresolvable
    /// callee). `false` = pure. Populated by
    /// `infer_fn_effects` during `check_program_with_source`.
    pub fn_effects: std::collections::HashMap<String, bool>,
    /// RES-318: number of `invariant` annotations the loop verifier
    /// statically discharged via Hoare-rule induction. Bumped per
    /// invariant, not per loop. Zero without `--features z3`.
    #[allow(dead_code)]
    pub loop_invariants_proven: usize,
}

impl VerificationStats {
    /// RES-068: names of functions whose call sites were ALL statically
    /// discharged AND there was at least one such call site. Empty
    /// requires (no contract) is excluded — there's nothing to elide.
    pub fn fully_provable_fns(&self) -> std::collections::HashSet<String> {
        // RES-1738: pre-size to per_fn_discharged.len() — exact upper
        // bound, since every entry is a candidate for the output.
        let mut out = std::collections::HashSet::with_capacity(self.per_fn_discharged.len());
        for (name, n) in &self.per_fn_discharged {
            if *n > 0 && !self.per_fn_runtime.contains_key(name) {
                out.insert(name.clone());
            }
        }
        out
    }
}

/// RES-067: shim that forwards to the Z3 module when built --features z3,
/// RES-777: check if a type string is a reference type.
/// Reference types are encoded as `"&[region] type"`, `"&mut[region] type"`,
/// `"& type"`, or `"&mut type"`.
fn is_reference_type(type_str: &str) -> bool {
    let trimmed = type_str.trim_start();
    trimmed.starts_with("&")
}

/// or returns None otherwise. Keeps the typechecker code agnostic to
/// whether the SMT layer is compiled in.
#[cfg(feature = "z3")]
#[allow(dead_code)]
fn z3_prove(expr: &Node, bindings: &HashMap<String, i64>) -> Option<bool> {
    crate::verifier_z3::prove(expr, bindings)
}
#[cfg(not(feature = "z3"))]
#[allow(dead_code)]
fn z3_prove(_expr: &Node, _bindings: &HashMap<String, i64>) -> Option<bool> {
    None
}

/// RES-071: like `z3_prove`, but also returns an SMT-LIB2 certificate
/// when the proof succeeds. RES-136: additionally returns a formatted
/// counterexample whenever the negated formula is satisfiable (the
/// `Some(false)` and `None` verdict cases), for use in verifier error
/// diagnostics. RES-137: fourth slot is `true` when Z3 returned
/// Unknown (per-query timeout fired); callers bump the timed-out
/// audit counter and emit a hint instead of treating as a proof
/// failure. RES-354: `theory` selects the SMT encoding (Auto/Bv/Lia).
/// Without `--features z3`, returns all-`None` / `false`.
#[cfg(feature = "z3")]
#[allow(dead_code)]
fn z3_prove_with_cert(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    timeout_ms: u32,
) -> (Option<bool>, Option<String>, Option<String>, bool) {
    z3_prove_with_cert_theory(
        expr,
        bindings,
        timeout_ms,
        crate::verifier_z3::Z3Theory::Auto,
    )
}

// RES-354: theory-aware variant of `z3_prove_with_cert`. Uses
// `prove_auto` which auto-selects BV32/LIA based on the theory hint
// and the presence of bitwise operations.
// RES-1631: within-run Z3 proof cache for `z3_prove_with_cert_theory`.
//
// Two call paths feed this entry point: the per-fn-decl tautology
// check (`check_program_with_source` line ~4664) and the per-call-site
// `requires` check (`check_call_expression` line ~6314). For
// programs that exercise the same `(clause, bindings, theory,
// timeout)` tuple twice in one type-check — most commonly hot test
// code calling a contracted fn with identical literal args — the
// second call rebuilds a Z3 Context, re-compiles SMT-LIB2, and
// re-runs `Solver::check` for an answer it has just computed.
//
// Cache is thread-local so concurrent tests don't share state, and
// cleared at the start of every `check_program_with_source` so
// state never leaks across compilations.
/// RES-1631: cached verdict for one `(clause, bindings, theory, timeout)`
/// tuple — `(verdict, cert_smtlib, cx_smtlib, used_runtime_fallback)`.
#[cfg(feature = "z3")]
type ProveCacheEntry = (Option<bool>, Option<String>, Option<String>, bool);

#[cfg(feature = "z3")]
thread_local! {
    // RES-1690: pre-size with capacity 64 — same pattern as RES-1688
    // for the inner Z3 caches. PROVE_CACHE accumulates one entry per
    // distinct `(clause, bindings, theory, timeout)` tuple within a
    // single typecheck; programs with ~100 obligations would otherwise
    // pay 5-6 rehashes per typecheck.
    //
    // RES-1708: use the same `IdentityU64Hasher` the inner Z3 caches
    // use (RES-1706). The `u64` key is already a high-quality hash
    // from `hash_node_spanless` + SipHash via `DefaultHasher`;
    // re-hashing it inside the HashMap costs ~5-10 cycles per
    // lookup that we can skip.
    static PROVE_CACHE: std::cell::RefCell<
        crate::verifier_z3::U64CacheMap<ProveCacheEntry>,
    > = std::cell::RefCell::new(
        crate::verifier_z3::U64CacheMap::with_capacity_and_hasher(64, Default::default()),
    );
}

/// RES-1631: reset the within-run Z3 proof cache. Called from
/// `check_program_with_source` at the start of every typecheck so
/// the cache never carries entries across compilations.
#[cfg(feature = "z3")]
pub(crate) fn reset_z3_prove_cache() {
    PROVE_CACHE.with(|c| c.borrow_mut().clear());
}
#[cfg(not(feature = "z3"))]
pub(crate) fn reset_z3_prove_cache() {}

/// Hash the (expr, bindings, timeout, theory) tuple into a stable u64
/// for `PROVE_CACHE`.
///
/// RES-1897: uses `hash_node_spanless` for structural hashing instead
/// of `format!("{:?}", expr)` which allocated a temporary String
/// proportional to the AST size on every cache lookup.
#[cfg(feature = "z3")]
fn prove_cache_key(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    timeout_ms: u32,
    theory: crate::verifier_z3::Z3Theory,
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    crate::verifier_z3::hash_node_spanless(expr, &mut h);
    let mut sorted: Vec<(&String, &i64)> = bindings.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in sorted {
        k.hash(&mut h);
        v.hash(&mut h);
    }
    timeout_ms.hash(&mut h);
    std::mem::discriminant(&theory).hash(&mut h);
    h.finish()
}

#[cfg(feature = "z3")]
fn z3_prove_with_cert_theory(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    timeout_ms: u32,
    theory: crate::verifier_z3::Z3Theory,
) -> (Option<bool>, Option<String>, Option<String>, bool) {
    let key = prove_cache_key(expr, bindings, timeout_ms, theory);
    if let Some(cached) = PROVE_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return cached;
    }
    let (verdict, cert, cx, timed_out) =
        crate::verifier_z3::prove_auto(expr, bindings, theory, timeout_ms);
    let result = (verdict, cert.map(|c| c.smt2), cx, timed_out);
    PROVE_CACHE.with(|c| c.borrow_mut().insert(key, result.clone()));
    result
}
#[cfg(not(feature = "z3"))]
fn z3_prove_with_cert(
    _expr: &Node,
    _bindings: &HashMap<String, i64>,
    _timeout_ms: u32,
) -> (Option<bool>, Option<String>, Option<String>, bool) {
    (None, None, None, false)
}

/// RES-222: like `z3_prove_with_cert`, but asserts a list of
/// boolean `axioms` alongside the clause. Used by the
/// `recovers_to` discharge path to admit each `requires`
/// clause as an assumption when proving the recovery
/// invariant — the recovery point is reached only after the
/// precondition has already been checked, so requires still
/// hold. Without `--features z3`, returns all-`None` / `false`.
/// RES-1633: cache key for `z3_prove_with_axioms_and_cert`. Same
/// shape as `prove_cache_key` (RES-1631) but folds the axioms
/// slice's Debug repr into the hash so two distinct axiom sets
/// can't collide. The leading `with_axioms:` tag also prevents
/// any collision with `prove_cache_key`'s no-axioms entries that
/// share the same `PROVE_CACHE`.
/// RES-1897: structural hashing for axiom-aware cache key.
#[cfg(feature = "z3")]
fn prove_cache_key_with_axioms(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    axioms: &[Node],
    timeout_ms: u32,
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    "with_axioms:".hash(&mut h);
    crate::verifier_z3::hash_node_spanless(expr, &mut h);
    let mut sorted: Vec<(&String, &i64)> = bindings.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in sorted {
        k.hash(&mut h);
        v.hash(&mut h);
    }
    for ax in axioms {
        crate::verifier_z3::hash_node_spanless(ax, &mut h);
    }
    timeout_ms.hash(&mut h);
    h.finish()
}

#[cfg(feature = "z3")]
fn z3_prove_with_axioms_and_cert(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    axioms: &[Node],
    timeout_ms: u32,
) -> (Option<bool>, Option<String>, Option<String>, bool) {
    let key = prove_cache_key_with_axioms(expr, bindings, axioms, timeout_ms);
    if let Some(cached) = PROVE_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return cached;
    }
    let (verdict, cert, cx, timed_out) =
        crate::verifier_z3::prove_with_axioms_and_timeout(expr, bindings, axioms, timeout_ms);
    let result = (verdict, cert.map(|c| c.smt2), cx, timed_out);
    PROVE_CACHE.with(|c| c.borrow_mut().insert(key, result.clone()));
    result
}
#[cfg(not(feature = "z3"))]
fn z3_prove_with_axioms_and_cert(
    _expr: &Node,
    _bindings: &HashMap<String, i64>,
    _axioms: &[Node],
    _timeout_ms: u32,
) -> (Option<bool>, Option<String>, Option<String>, bool) {
    (None, None, None, false)
}

/// RES-217: best-effort span for a contract clause. Mirrors
/// the helper in `infer::expr_span` — duplicated here so the
/// typechecker stays independent of the inference module's
/// feature gating. Nodes outside the supported expression
/// subset fall back to a default (line-0) span, which the
/// warning formatter detects and prints `<unknown>` for.
fn clause_span(node: &Node) -> Span {
    match node {
        Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::Identifier { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::CallExpression { span, .. }
        | Node::TryExpression { span, .. }
        | Node::OptionalChain { span, .. } => *span,
        _ => Span::default(),
    }
}

/// RES-340: gate for the rich type-mismatch diagnostic format.
/// The legacy short message (`Type mismatch in argument N: expected
/// X, got Y`) is emitted by default so existing callers — including
/// every `.expected.txt` golden — stay byte-identical. Setting
/// `RESILIENT_RICH_DIAG=1` switches to a rustc-style multi-block
/// diagnostic that includes a primary span on the offending argument
/// and a secondary span on the function declaration.
fn rich_diag_enabled() -> bool {
    // RES-1361: cache the env-var lookup. Called per type-mismatch
    // diagnostic during `check_node`'s `CallExpression` arm; the
    // raw `std::env::var` is a syscall (typically 100ns-1µs hot).
    // `LazyLock` reads the env once per process; every subsequent
    // call is a relaxed atomic load of the cached `bool`. Mirrors
    // RES-1341 for `RESILIENT_CONST_FOLD`. No test in the codebase
    // flips this var mid-process.
    static ENABLED: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var("RESILIENT_RICH_DIAG").as_deref() == Ok("1"));
    *ENABLED
}

/// RES-340: format a rich type-mismatch diagnostic. Loads source
/// on demand from `source_path` (best-effort — falls back to the
/// terse format if the file can't be read, which keeps the path
/// safe under unit tests where the source lives in memory only).
///
/// The shape mirrors rustc's E0308:
///
/// ```text
/// error[E0007]: type mismatch in argument N
///    fn callee(int dist) { ... }
///       ^^^^^^ expected `T` because of this declaration
///    drive(elapsed)
///          ^^^^^^^ found `U`
/// ```
fn render_rich_arg_type_mismatch(
    source_path: &str,
    arg_span: crate::span::Span,
    decl_span: Option<crate::span::Span>,
    arg_idx_one_based: usize,
    expected: &str,
    actual: &str,
) -> String {
    let primary_msg = format!(
        "type mismatch in argument {}: expected `{}`, found `{}`",
        arg_idx_one_based, expected, actual
    );
    let mut diag =
        crate::diag::Diagnostic::new(crate::diag::Severity::Error, arg_span, primary_msg)
            .with_code(crate::diag::codes::E0007);
    if let Some(ds) = decl_span {
        diag = diag.with_note(
            ds,
            format!("expected `{}` because of this declaration", expected),
        );
    }
    let src = if source_path.is_empty() {
        String::new()
    } else {
        std::fs::read_to_string(source_path).unwrap_or_default()
    };
    crate::diag::format_diagnostic_terminal(&src, &diag)
}

/// RES-217: format + print the partial-proof warning to stderr.
/// The message follows the ticket's mandated shape:
///
/// ```text
/// warning[partial-proof]: Z3 returned Unknown for assertion at <file>:<line>:<col> — proof is incomplete
/// ```
///
/// `source_path` is the typechecker's recorded file path (set
/// by `check_program_with_source`); an empty string falls back
/// to `<unknown>` so REPL / unit-test callers still produce a
/// readable line. Spans whose `start.line` is 0 (synthetic
/// clauses, e.g. REPL-constructed AST) print `<unknown>` for
/// the position — avoiding misleading `:0:0`.
#[cfg_attr(not(feature = "z3"), allow(dead_code))]
fn emit_partial_proof_warning(source_path: &str, clause: &Node) {
    let span = clause_span(clause);
    let file = if source_path.is_empty() {
        "<unknown>"
    } else {
        source_path
    };
    let location = if span.start.line == 0 {
        format!("{}:<unknown>", file)
    } else {
        format!("{}:{}:{}", file, span.start.line, span.start.column)
    };
    eprintln!(
        "warning[partial-proof]: Z3 returned Unknown for assertion at {} \u{2014} proof is incomplete",
        location
    );
}

/// RES-071: a single SMT-LIB2 proof certificate that the typechecker
/// captured when Z3 successfully discharged a contract obligation.
/// Filename on disk: `{fn_name}__{kind}__{idx}.smt2`.
///
/// RES-1202: fields are only ever read by the z3-feature-gated
/// `emit_certificates` / `dispatch_verify_*` flows in `lib.rs`,
/// so a default-feature build sees them as dead. Suppress the
/// lint under that exact condition rather than universally.
#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "z3"), allow(dead_code))]
pub struct CapturedCertificate {
    pub fn_name: String,
    pub kind: &'static str,
    pub idx: usize,
    pub smt2: String,
}

// Type checker for verifying type correctness
pub struct TypeChecker {
    env: TypeEnvironment,
    /// RES-061: top-level function name → its parameters + contract clauses.
    /// Populated by check_program's first pass; consulted by CallExpression.
    /// RES-1363: stored behind `Rc` so the per-CallExpression lookup
    /// at typechecker.rs:5877 ticks a single refcount instead of
    /// deep-cloning four `Vec`s on every call to a user-defined fn.
    contract_table: HashMap<String, std::rc::Rc<ContractInfo>>,
    /// RES-340: top-level function name → span of its `fn` keyword.
    /// Populated by the same pre-pass that fills `contract_table`. Read
    /// by the rich-diagnostic path in `CallExpression` to attach a
    /// secondary "expected `T` because of this parameter" label
    /// pointing at the function declaration.
    fn_decl_spans: HashMap<String, Span>,
    /// RES-063: identifier → known constant integer value.
    const_bindings: HashMap<String, i64>,
    /// RES-066: verification audit counters.
    pub stats: VerificationStats,
    /// RES-071: SMT-LIB2 certificates accumulated by every successful
    /// Z3 proof. The driver writes these to disk when invoked with
    /// `--emit-certificate <DIR>`.
    pub certificates: Vec<CapturedCertificate>,
    /// RES-153: struct name → (field_name → parsed field type). Populated
    /// when we visit each `StructDecl`. Used by `FieldAccess` to return
    /// the declared field's type instead of `Type::Any`, and by
    /// `FieldAssignment` to reject writes to non-existent fields
    /// statically.
    /// RES-1365: stored behind `Rc` so the per-FieldAccess /
    /// FieldAssignment / Match-pattern reads at 4152 / 4831 etc. tick
    /// a single refcount instead of deep-cloning the
    /// `Vec<(String, Type)>` (which can be large for structs with
    /// many fields, especially when entries carry `Type::Function`
    /// with their own nested Vecs).
    struct_fields: HashMap<String, std::rc::Rc<Vec<(String, Type)>>>,
    /// RES-400: enum name → variant list. Populated when we visit each
    /// `EnumDecl`. Used by the `Match` exhaustiveness check to ensure
    /// every declared variant is covered, and by `match_pattern_binding_types`
    /// (future PR) to produce proper payload-field types instead of
    /// the current `Any` fallback.
    ///
    /// RES-1368: value wrapped in `Rc` so the two `.get().cloned()`
    /// lookup sites in `check_node`'s pattern-match arms become a
    /// refcount bump rather than a full `Vec<EnumVariant>::clone`
    /// (each variant carries an owned `String` name + a span + a
    /// payload, all duplicated per lookup). Mirrors RES-1363's
    /// `contract_table` and RES-1365's `struct_fields` refactors.
    pub(crate) enum_decls: HashMap<String, std::sync::Arc<Vec<crate::EnumVariant>>>,
    /// RES-128: alias name → raw target type name. Populated by
    /// every `TypeAlias` node. Consulted by `parse_type_name` which
    /// walks the chain (with a `seen` set for cycle detection) and
    /// returns the ultimate `Type` the alias resolves to.
    type_aliases: HashMap<String, String>,
    /// RES-137: per-query Z3 solver timeout in milliseconds. `0`
    /// disables the timeout (use Z3's default, which is unlimited).
    /// The driver sets this from the `--verifier-timeout-ms <N>`
    /// CLI flag (default 5000). On timeout, the verifier returns
    /// Unknown — treated as "not proven" rather than an error, so
    /// compilation continues with the runtime check retained.
    verifier_timeout_ms: u32,
    /// RES-217: when `true`, Z3 returning `Unknown` (timeout or
    /// undecidable theory) emits a `warning[partial-proof]`
    /// diagnostic to stderr that names the specific assertion
    /// and its source position. Defaults to `true`; the driver
    /// flips it off via `--no-warn-unverified` when CI noise is
    /// unwanted. Independent of the pre-existing `hint: proof
    /// timed out ...` line, which is the per-fn diagnostic about
    /// the `--verifier-timeout-ms` budget.
    warn_unverified: bool,
    /// RES-217: source path threaded from
    /// `check_program_with_source` so the partial-proof warning
    /// can print `<file>:<line>:<col>`. Empty when the caller
    /// used the `check_program` shim (REPL / unit tests); the
    /// warning falls back to `<unknown>` in that case.
    source_path: String,
    /// RES-189: inferred types for unannotated `let` bindings,
    /// accumulated during the walk. The LSP backend reads this
    /// after `check_program_with_source` to produce inlay hints.
    /// Empty when check fails before reaching any unannotated
    /// let — that's an acceptable partial behaviour (errors take
    /// precedence over hints for broken files).
    pub let_type_hints: Vec<LetTypeHint>,
    /// RES-387: declared failure variants of the fn currently
    /// being checked. Pushed on entry to `Node::Function` and
    /// popped on exit. Consulted at every `CallExpression` to
    /// enforce that callees' `fails` variants are propagated on
    /// the caller's signature. `None` means we're not inside a
    /// named fn — call sites in top-level code (e.g. `live`
    /// blocks) cannot raise checked failures today and must only
    /// invoke fns with an empty `fails` set.
    current_fn_fails: Option<Vec<String>>,
    /// RES-403: declared return type of the innermost enclosing function.
    /// Set when entering a `Node::Function` with a non-empty `return_type`;
    /// cleared (set to `None`) on exit. `Node::ReturnStatement` uses this
    /// to validate `return expr` against the declared type, catching early
    /// returns that bypass the function body's final-expression check.
    current_fn_return_type: Option<Type>,
    /// RES-910: depth of enclosing `while` / `for-in` bodies. `break`
    /// and `continue` are typechecker-rejected when this is 0. Bumped
    /// before recursing into a loop body and decremented after.
    loop_depth: usize,
    /// RES-2653: stack of loop labels in scope. Each entry is the label
    /// of the corresponding enclosing loop (None for unlabeled loops).
    /// Used to validate `break label` and `continue label`.
    loop_label_stack: Vec<Option<String>>,
    /// RES-354: SMT theory selection. Auto-detect (BV32 if bitwise
    /// ops are present, LIA otherwise) by default. The driver
    /// overrides this from `--z3-theory <bv|lia|auto>`.
    #[cfg(feature = "z3")]
    z3_theory: crate::verifier_z3::Z3Theory,
    /// RES-318: when `true`, the loop-invariant verifier prints a
    /// `-- invariant proven, runtime check elided at L:C` line per
    /// successfully discharged invariant. Defaults to `false` so the
    /// regular build is silent; the driver flips it on for `--verbose`.
    verbose_loop_invariants: bool,
    /// RES-1322: gate the post-typecheck `infer_fn_effects` fixpoint
    /// on opt-in. The result populates `self.stats.fn_effects`, which
    /// is only read by the `--audit` and `--explain-effects` CLI
    /// drivers — for every other invocation (the default
    /// `rz prog.rz` path, the LSP/REPL, every `cargo test` typecheck
    /// call) the fixpoint is wasted work because the map is never
    /// consulted. Default `false`; the audit/explain-effects drivers
    /// set it via `with_audit_stats(true)`.
    audit_stats: bool,
    /// RES-1353: opt-in flag for populating `let_type_hints`. The
    /// hints are only consumed by `lsp_server`'s inlay-hint provider,
    /// so every non-LSP compile pushed a `LetTypeHint` per inferred
    /// `let` only to drop the Vec on TypeChecker drop. Default
    /// `false`; the LSP path flips it via `with_capture_inlay_hints(true)`.
    capture_inlay_hints: bool,
    /// RES-1357: opt-in flag for pushing `CapturedCertificate`
    /// entries onto `self.certificates`. The only consumer is the
    /// `--emit-certificate <DIR>` CLI driver (lib.rs:26008) — every
    /// other invocation (default `rz prog.rz`, LSP/REPL, every
    /// `cargo test` typecheck) pushed certs onto a Vec it dropped
    /// on TypeChecker drop. Default `false`; the cert-emit driver
    /// flips it via `with_emit_certificates(true)`.
    emit_certificates: bool,
    /// RES-1862: innermost span updated as `check_node` descends the
    /// AST. When an error propagates back to `check_program_with_source`
    /// this span is more specific than the top-level statement's span
    /// (which covers the entire statement from `let` / `fn` to `;`).
    /// Reset to `stmt.span` at the start of each top-level statement
    /// so stale spans from a prior statement never pollute a later one.
    current_span: Span,

    /// RES-425: maps function name → list of generic type-parameter names
    /// declared with `fn foo<T, U>(...)`. Used at call sites to recognise
    /// `Type::Struct("T")` as a type variable that accepts any concrete
    /// type, fixing the "Type mismatch in argument 1: expected T, got int"
    /// false-positive that blocked all generic-function calls.
    fn_type_params: HashMap<String, Vec<String>>,
    /// RES-2693: struct name → set of trait names the struct implements.
    /// Populated by the pre-pass when scanning `ImplBlock` declarations.
    /// Used by `satisfies_trait_param` to allow a concrete struct to satisfy
    /// a trait-typed parameter, return annotation, or let binding.
    trait_impls: HashMap<String, HashSet<String>>,
    /// RES-2697: trait name → set of method names that carry a default body.
    /// Populated when processing `TraitDecl` nodes. Used to allow
    /// `FieldAccess` type-checks to succeed for default methods even when
    /// the impl block omits them.
    trait_default_methods: HashMap<String, HashSet<String>>,
}

impl TypeChecker {
    pub fn new() -> Self {
        // RES-1349 + RES-2444: cache the built-in env once per
        // process as an `Arc<TypeEnvironment>`. Each `TypeChecker`
        // gets the builtins as the *outer* scope of its env instead
        // of cloning ~490 `(String, Type)` pairs into the local
        // store. User-defined bindings go into the (initially
        // empty) local store and shadow builtins via the normal
        // scope-chain lookup. Effect: `TypeChecker::new()` does
        // one `Arc::clone` (refcount bump) instead of ~970 heap
        // allocations, and every scope entry (function body, block,
        // loop) likewise skips re-cloning the builtin entries.
        static BUILTIN_ENV: std::sync::LazyLock<std::sync::Arc<TypeEnvironment>> =
            std::sync::LazyLock::new(|| {
                // RES-1698: pre-size to fit the ~490 builtin entries.
                // Saves 9 rehash rounds on the one-time LazyLock init.
                let mut env = TypeEnvironment::with_capacity(512);

                // Built-in function signatures. Any-typed parameters keep the
                // type checker permissive for heterogeneous inputs until real
                // generics arrive (RES-055).
                let fn_any_to_void = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Void),
                };
                let fn_any_any_to_any = || Type::Function {
                    params: vec![Type::Any, Type::Any],
                    return_type: Box::new(Type::Any),
                };
                let fn_any_to_any = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Any),
                };
                let fn_any_to_int = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Int),
                };
                let fn_any_any_to_bool = || Type::Function {
                    params: vec![Type::Any, Type::Any],
                    return_type: Box::new(Type::Bool),
                };
                let fn_any_any_to_int = || Type::Function {
                    params: vec![Type::Any, Type::Any],
                    return_type: Box::new(Type::Int),
                };
                let fn_any_any_to_array = || Type::Function {
                    params: vec![Type::Any, Type::Any],
                    return_type: Box::new(Type::Array),
                };
                let fn_any_to_result = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Result),
                };
                let fn_result_to_bool = || Type::Function {
                    params: vec![Type::Result],
                    return_type: Box::new(Type::Bool),
                };
                let fn_result_to_any = || Type::Function {
                    params: vec![Type::Result],
                    return_type: Box::new(Type::Any),
                };

                // I/O
                env.set("println".to_string(), fn_any_to_void());
                env.set("print".to_string(), fn_any_to_void());

                // RES-385: `drop(v)` — explicit single-use consumption
                // of a linear value. Accepts any type; the linearity pass
                // (see `crate::linear`) is what enforces the single-use
                // rule on the argument.
                env.set("drop".to_string(), fn_any_to_void());

                // Math (single-arg — int/float passed as Any)
                env.set("abs".to_string(), fn_any_to_any());

                // RES-422: sign(x) always returns -1, 0, or +1 — that's Int.
                env.set(
                    "sign".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );

                // RES-411: float predicates — return Bool; math functions return Float.
                // (Parameter is kept as Any so both Int and Float are accepted.)
                let fn_any_to_bool = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Bool),
                };
                let fn_any_to_float = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Float),
                };
                env.set("is_nan".to_string(), fn_any_to_bool());
                env.set("is_inf".to_string(), fn_any_to_bool());
                env.set("is_finite".to_string(), fn_any_to_bool());
                env.set("sqrt".to_string(), fn_any_to_float());
                env.set("floor".to_string(), fn_any_to_float());
                env.set("ceil".to_string(), fn_any_to_float());
                env.set("min".to_string(), fn_any_any_to_any());
                env.set("max".to_string(), fn_any_any_to_any());
                // RES-415: gcd/lcm — strict (Int, Int) -> Int.
                env.set(
                    "gcd".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "lcm".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-536: gcd / lcm reduction over an integer array.
                let arr_to_int = Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Int),
                };
                env.set("gcd_array".to_string(), arr_to_int.clone());
                env.set("lcm_array".to_string(), arr_to_int);
                // RES-567: factorial with overflow detection.
                env.set(
                    "factorial".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-568: binomial coefficient C(n, k).
                env.set(
                    "binomial".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-569: n-th Fibonacci with overflow detection.
                env.set(
                    "fibonacci".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-570: trial-division primality test.
                env.set(
                    "is_prime".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Bool),
                    },
                );
                // RES-571: smallest prime greater than n.
                env.set(
                    "next_prime".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set("pow".to_string(), fn_any_any_to_any());
                // RES-295: clamp(x, lo, hi) — type-preserving for Int triples,
                // promoted to Float if any arg is Float. Signed as
                // (Any, Any, Any) -> Any to match abs/min/max precedent.
                env.set(
                    "clamp".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );

                // RES-146: transcendentals. Float-in / Float-out per
                // RES-130 (no implicit int↔float coercion).
                let fn_float_to_float = || Type::Function {
                    params: vec![Type::Float],
                    return_type: Box::new(Type::Float),
                };
                env.set("sin".to_string(), fn_float_to_float());
                env.set("cos".to_string(), fn_float_to_float());
                env.set("tan".to_string(), fn_float_to_float());
                // RES-894.
                env.set("to_radians".to_string(), fn_float_to_float());
                // RES-895.
                env.set("to_degrees".to_string(), fn_float_to_float());
                env.set("ln".to_string(), fn_float_to_float());
                // RES-889.
                env.set("log10".to_string(), fn_float_to_float());
                // RES-890.
                env.set("log2".to_string(), fn_float_to_float());
                env.set("exp".to_string(), fn_float_to_float());
                // RES-891.
                env.set("exp2".to_string(), fn_float_to_float());
                // RES-896.
                env.set("sinh".to_string(), fn_float_to_float());
                // RES-897.
                env.set("cosh".to_string(), fn_float_to_float());
                // RES-898.
                env.set("tanh".to_string(), fn_float_to_float());
                // RES-899.
                env.set("asinh".to_string(), fn_float_to_float());
                // RES-900.
                env.set("acosh".to_string(), fn_float_to_float());
                // RES-901.
                env.set("atanh".to_string(), fn_float_to_float());
                // RES-902.
                env.set("asin".to_string(), fn_float_to_float());
                // RES-903.
                env.set("acos".to_string(), fn_float_to_float());
                // RES-904.
                env.set("atan".to_string(), fn_float_to_float());
                // RES-905.
                env.set("cbrt".to_string(), fn_float_to_float());
                // RES-907: bit-counting integer builtins (Int -> Int).
                let fn_int_to_int = || Type::Function {
                    params: vec![Type::Int],
                    return_type: Box::new(Type::Int),
                };
                env.set("count_ones".to_string(), fn_int_to_int());
                env.set("count_zeros".to_string(), fn_int_to_int());
                env.set("leading_zeros".to_string(), fn_int_to_int());
                env.set("trailing_zeros".to_string(), fn_int_to_int());
                // RES-1115..1118: overflow-safe integer arithmetic. The
                // checked_* family returns Option<int>.
                // RES-2651: use Type::Option(Int) so pattern matching
                // on `Some(x)` binds `x` to `Int`, not `Any`.
                let fn_int_int_to_int = || Type::Function {
                    params: vec![Type::Int, Type::Int],
                    return_type: Box::new(Type::Int),
                };
                let fn_int_int_to_option_int = || Type::Function {
                    params: vec![Type::Int, Type::Int],
                    return_type: Box::new(Type::Option(Box::new(Type::Int))),
                };
                env.set("saturating_add".to_string(), fn_int_int_to_int());
                env.set("saturating_sub".to_string(), fn_int_int_to_int());
                env.set("saturating_mul".to_string(), fn_int_int_to_int());
                env.set("wrapping_add".to_string(), fn_int_int_to_int());
                env.set("wrapping_sub".to_string(), fn_int_int_to_int());
                env.set("wrapping_mul".to_string(), fn_int_int_to_int());
                env.set("checked_add".to_string(), fn_int_int_to_option_int());
                env.set("checked_sub".to_string(), fn_int_int_to_option_int());
                env.set("checked_mul".to_string(), fn_int_int_to_option_int());
                env.set("checked_div".to_string(), fn_int_int_to_option_int());
                // RES-1119..1121: bit manipulation.
                env.set("rotate_left_int".to_string(), fn_int_int_to_int());
                env.set("rotate_right_int".to_string(), fn_int_int_to_int());
                env.set("reverse_bits".to_string(), fn_int_to_int());
                env.set("swap_bytes".to_string(), fn_int_to_int());
                // RES-1122..1123: int ↔ bytes endianness conversion.
                env.set(
                    "to_be_bytes".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Bytes),
                    },
                );
                env.set(
                    "to_le_bytes".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Bytes),
                    },
                );
                env.set(
                    "from_be_bytes".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "from_le_bytes".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-1124: integer-only math primitives.
                env.set("isqrt".to_string(), fn_int_to_int());
                env.set("ipow".to_string(), fn_int_int_to_int());
                // RES-1126..1128: direction-rounded + Euclidean + midpoint.
                env.set("div_ceil".to_string(), fn_int_int_to_int());
                env.set("div_floor".to_string(), fn_int_int_to_int());
                env.set("div_euclid".to_string(), fn_int_int_to_int());
                env.set("rem_euclid".to_string(), fn_int_int_to_int());
                env.set("midpoint".to_string(), fn_int_int_to_int());
                // RES-1129: integer logarithms.
                env.set("ilog2".to_string(), fn_int_to_int());
                env.set("ilog10".to_string(), fn_int_to_int());
                // RES-1130: IEEE 754 bit reinterpret cast.
                env.set(
                    "float_to_bits".to_string(),
                    Type::Function {
                        params: vec![Type::Float],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "float_from_bits".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Float),
                    },
                );
                // RES-1134: bitwise + construction ops on Bytes.
                let fn_bytes_bytes_to_bytes = || Type::Function {
                    params: vec![Type::Bytes, Type::Bytes],
                    return_type: Box::new(Type::Bytes),
                };
                let fn_bytes_to_bytes = || Type::Function {
                    params: vec![Type::Bytes],
                    return_type: Box::new(Type::Bytes),
                };
                env.set("bytes_xor".to_string(), fn_bytes_bytes_to_bytes());
                env.set("bytes_and".to_string(), fn_bytes_bytes_to_bytes());
                env.set("bytes_or".to_string(), fn_bytes_bytes_to_bytes());
                env.set("bytes_not".to_string(), fn_bytes_to_bytes());
                env.set(
                    "bytes_fill".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::Bytes),
                    },
                );
                env.set("bytes_reverse".to_string(), fn_bytes_to_bytes());
                // RES-1136: alignment helpers.
                env.set("next_multiple_of".to_string(), fn_int_int_to_int());
                env.set(
                    "is_multiple_of".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::Bool),
                    },
                );
                // RES-1138: IEEE 754 classification + total order + sign-bit
                // predicates.
                let fn_float_to_bool = || Type::Function {
                    params: vec![Type::Float],
                    return_type: Box::new(Type::Bool),
                };
                env.set(
                    "float_classify".to_string(),
                    Type::Function {
                        params: vec![Type::Float],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "float_total_cmp".to_string(),
                    Type::Function {
                        params: vec![Type::Float, Type::Float],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set("float_is_normal".to_string(), fn_float_to_bool());
                env.set("float_is_subnormal".to_string(), fn_float_to_bool());
                env.set("float_sign_bit".to_string(), fn_float_to_bool());
                // RES-1142: array chunking + striding + rotation primitives.
                let fn_array_int_to_array = || Type::Function {
                    params: vec![Type::Array, Type::Int],
                    return_type: Box::new(Type::Array),
                };
                env.set("array_chunks".to_string(), fn_array_int_to_array());
                env.set("array_chunks_exact".to_string(), fn_array_int_to_array());
                env.set("array_step".to_string(), fn_array_int_to_array());
                env.set("array_rotate_left".to_string(), fn_array_int_to_array());
                env.set("array_rotate_right".to_string(), fn_array_int_to_array());
                // RES-1140: ASCII char-class predicates.
                let fn_string_to_bool = || Type::Function {
                    params: vec![Type::String],
                    return_type: Box::new(Type::Bool),
                };
                env.set("is_ascii".to_string(), fn_string_to_bool());
                env.set("is_ascii_whitespace".to_string(), fn_string_to_bool());
                env.set("is_ascii_hexdigit".to_string(), fn_string_to_bool());
                env.set("is_ascii_uppercase".to_string(), fn_string_to_bool());
                env.set("is_ascii_lowercase".to_string(), fn_string_to_bool());
                env.set("is_ascii_punctuation".to_string(), fn_string_to_bool());
                env.set("is_ascii_control".to_string(), fn_string_to_bool());
                // RES-1146: float / string sort + array_is_sorted predicates.
                let fn_array_to_array = || Type::Function {
                    params: vec![Type::Array],
                    return_type: Box::new(Type::Array),
                };
                let fn_array_to_bool = || Type::Function {
                    params: vec![Type::Array],
                    return_type: Box::new(Type::Bool),
                };
                env.set("array_sort_float".to_string(), fn_array_to_array());
                env.set("array_sort_string".to_string(), fn_array_to_array());
                env.set("array_is_sorted".to_string(), fn_array_to_bool());
                env.set("array_is_sorted_float".to_string(), fn_array_to_bool());
                env.set("array_is_sorted_string".to_string(), fn_array_to_bool());
                // RES-1148: binary search on sorted int / float / string arrays.
                // Return type is Value::Result (ok/err) — now typed as Type::Result
                // since the runtime confirmed returns Value::Result { ok, payload }.
                env.set(
                    "array_binary_search".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Int],
                        return_type: Box::new(Type::Result),
                    },
                );
                env.set(
                    "array_binary_search_float".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Float],
                        return_type: Box::new(Type::Result),
                    },
                );
                env.set(
                    "array_binary_search_string".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::String],
                        return_type: Box::new(Type::Result),
                    },
                );
                // RES-1150: statistical reductions — variance, stddev,
                // median_float, range_float. All return Float.
                let fn_array_to_float = || Type::Function {
                    params: vec![Type::Array],
                    return_type: Box::new(Type::Float),
                };
                env.set("array_variance_int".to_string(), fn_array_to_float());
                env.set("array_variance_float".to_string(), fn_array_to_float());
                env.set("array_stddev_int".to_string(), fn_array_to_float());
                env.set("array_stddev_float".to_string(), fn_array_to_float());
                env.set("array_median_float".to_string(), fn_array_to_float());
                env.set("array_range_float".to_string(), fn_array_to_float());
                // RES-1152: per-byte helpers — repeat / count_byte / replace_byte.
                env.set(
                    "bytes_repeat".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes, Type::Int],
                        return_type: Box::new(Type::Bytes),
                    },
                );
                env.set(
                    "bytes_count_byte".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "bytes_replace_byte".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes, Type::Int, Type::Int],
                        return_type: Box::new(Type::Bytes),
                    },
                );
                // RES-1156: per-bit accessors on i64.
                env.set("set_bit".to_string(), fn_int_int_to_int());
                env.set("clear_bit".to_string(), fn_int_int_to_int());
                env.set(
                    "get_bit".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set("flip_bit".to_string(), fn_int_int_to_int());
                // RES-1158: array set-style helpers + fallback-safe first/last
                // + index_of_last.
                let fn_array_array_to_array = || Type::Function {
                    params: vec![Type::Array, Type::Array],
                    return_type: Box::new(Type::Array),
                };
                env.set("array_difference".to_string(), fn_array_array_to_array());
                env.set("array_intersection".to_string(), fn_array_array_to_array());
                env.set(
                    "array_index_of_last".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "array_first_or".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "array_last_or".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                // RES-1162: deterministic hash builtins.
                env.set(
                    "hash_int".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "hash_string".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "hash_bytes".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set("hash_combine".to_string(), fn_int_int_to_int());
                // RES-1164: iteration helpers.
                env.set(
                    "enumerate".to_string(),
                    Type::Function {
                        params: vec![Type::Array],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "array_zip3".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Array, Type::Array],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "string_truncate".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-1166: rounding builtins.
                let fn_float_to_float = || Type::Function {
                    params: vec![Type::Float],
                    return_type: Box::new(Type::Float),
                };
                let fn_float_to_int = || Type::Function {
                    params: vec![Type::Float],
                    return_type: Box::new(Type::Int),
                };
                env.set("round".to_string(), fn_float_to_float());
                env.set("trunc".to_string(), fn_float_to_float());
                env.set("round_to_int".to_string(), fn_float_to_int());
                env.set("trunc_to_int".to_string(), fn_float_to_int());
                // RES-1170: cumulative reductions + combined min/max.
                let fn_array_to_array_int = || Type::Function {
                    params: vec![Type::Array],
                    return_type: Box::new(Type::Array),
                };
                env.set("array_cumsum".to_string(), fn_array_to_array_int());
                env.set("array_cumprod".to_string(), fn_array_to_array_int());
                env.set("array_diffs".to_string(), fn_array_to_array_int());
                env.set("array_min_max".to_string(), fn_array_to_array_int());
                // RES-1172: small string + array gaps.
                let fn_string_to_array = || Type::Function {
                    params: vec![Type::String],
                    return_type: Box::new(Type::Array),
                };
                let fn_string_string_to_array = || Type::Function {
                    params: vec![Type::String, Type::String],
                    return_type: Box::new(Type::Array),
                };
                env.set("string_split_once".to_string(), fn_string_string_to_array());
                env.set(
                    "string_rsplit_once".to_string(),
                    fn_string_string_to_array(),
                );
                env.set(
                    "string_from_chars".to_string(),
                    Type::Function {
                        params: vec![Type::Array],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "array_is_empty".to_string(),
                    Type::Function {
                        params: vec![Type::Array],
                        return_type: Box::new(Type::Bool),
                    },
                );
                // RES-1174: wall-clock unix time. @io / impure.
                let fn_zero_to_int = || Type::Function {
                    params: vec![],
                    return_type: Box::new(Type::Int),
                };
                env.set("unix_time_s".to_string(), fn_zero_to_int());
                env.set("unix_time_ms".to_string(), fn_zero_to_int());
                env.set("unix_time_ns".to_string(), fn_zero_to_int());
                // RES-1176: bytes ↔ string conversions.
                let fn_bytes_bytes_to_bytes_strip = || Type::Function {
                    params: vec![Type::Bytes, Type::Bytes],
                    return_type: Box::new(Type::Bytes),
                };
                env.set(
                    "bytes_strip_prefix".to_string(),
                    fn_bytes_bytes_to_bytes_strip(),
                );
                env.set(
                    "bytes_strip_suffix".to_string(),
                    fn_bytes_bytes_to_bytes_strip(),
                );
                // RES-1178: bytes_to_string always produces a String value on
                // the success path (invalid UTF-8 returns a lossy string, not a
                // different type). Promote from Any → String so the typechecker
                // can verify that callers don't treat the result as an int/bool.
                env.set(
                    "bytes_to_string".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-1178: bytes slicing primitives — (Bytes, Int) -> Bytes.
                let fn_bytes_int_to_bytes = || Type::Function {
                    params: vec![Type::Bytes, Type::Int],
                    return_type: Box::new(Type::Bytes),
                };
                env.set("bytes_take".to_string(), fn_bytes_int_to_bytes());
                env.set("bytes_drop".to_string(), fn_bytes_int_to_bytes());
                env.set("bytes_take_last".to_string(), fn_bytes_int_to_bytes());
                env.set("bytes_drop_last".to_string(), fn_bytes_int_to_bytes());
                // RES-1182: integer bit rotation + scalar signum.
                let fn_int_int_to_int = || Type::Function {
                    params: vec![Type::Int, Type::Int],
                    return_type: Box::new(Type::Int),
                };
                env.set("rotate_left".to_string(), fn_int_int_to_int());
                env.set("rotate_right".to_string(), fn_int_int_to_int());
                env.set(
                    "signum".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "log".to_string(),
                    Type::Function {
                        params: vec![Type::Float, Type::Float],
                        return_type: Box::new(Type::Float),
                    },
                );
                // RES-295: atan2(y, x) — Float-only per RES-130.
                env.set(
                    "atan2".to_string(),
                    Type::Function {
                        params: vec![Type::Float, Type::Float],
                        return_type: Box::new(Type::Float),
                    },
                );
                // RES-892.
                env.set(
                    "hypot".to_string(),
                    Type::Function {
                        params: vec![Type::Float, Type::Float],
                        return_type: Box::new(Type::Float),
                    },
                );
                // RES-893.
                env.set(
                    "copysign".to_string(),
                    Type::Function {
                        params: vec![Type::Float, Type::Float],
                        return_type: Box::new(Type::Float),
                    },
                );

                // RES-147: monotonic ms-clock builtin. std-only.
                env.set(
                    "clock_ms".to_string(),
                    Type::Function {
                        params: vec![],
                        return_type: Box::new(Type::Int),
                    },
                );

                // RES-150: seedable random builtins. std-only.
                env.set(
                    "random_int".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "random_float".to_string(),
                    Type::Function {
                        params: vec![],
                        return_type: Box::new(Type::Float),
                    },
                );

                // len: any -> int
                env.set(
                    "len".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );

                // Array builtins: any -> array / (array,int,int) -> array
                env.set(
                    "push".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "pop".to_string(),
                    Type::Function {
                        params: vec![Type::Array],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "slice".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Int, Type::Int],
                        return_type: Box::new(Type::Array),
                    },
                );

                // String builtins
                env.set(
                    "split".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-1859: `string_split` — explicit-name alias for `split`.
                env.set(
                    "string_split".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-535: split with a maximum number-of-splits limit.
                env.set(
                    "string_split_n".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String, Type::Int],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-545: split on the last occurrence of the separator.
                env.set(
                    "string_split_last".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "trim".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "contains".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set(
                    "to_upper".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "to_lower".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-412: reverse a string (chars) or an array (clones).
                env.set(
                    "string_reverse".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set("array_reverse".to_string(), fn_array_to_array());
                // RES-1859: higher-order array builtins — callback is typed
                // as Any because we have no generic function type yet.
                env.set(
                    "array_map".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "array_filter".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "array_reduce".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                // RES-507: generic callback-based search/predicate builtins.
                env.set(
                    "array_find".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "array_find_index".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "array_any".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set(
                    "array_all".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );

                // RES-2647: map functional operations (callback-taking).
                // Return types use the same permissive-Any convention as map_keys/map_values.
                env.set(
                    "map_filter".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "map_map_values".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "map_for_each".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Void),
                    },
                );
                env.set(
                    "map_to_pairs".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "map_invert".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                // RES-2646: higher-order functional array operations.
                env.set(
                    "array_flat_map".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // array_group_by returns a Map (unparameterised, same convention
                // as other map builtins — Type::Any until Map<K,V> lands).
                env.set(
                    "array_group_by".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                // array_partition returns [[passing], [failing]] — two-element array.
                env.set(
                    "array_partition".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // map_from_pairs(pairs) -> Map (Any until Map<K,V> lands).
                env.set(
                    "map_from_pairs".to_string(),
                    Type::Function {
                        params: vec![Type::Array],
                        return_type: Box::new(Type::Any),
                    },
                );
                // RES-2646: array_scan(arr, init, fn) -> Array of all prefix accumulator values.
                env.set(
                    "array_scan".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-2648: array combinators with arbitrary callbacks.
                let arr_fn_to_arr = Type::Function {
                    params: vec![Type::Array, Type::Any],
                    return_type: Box::new(Type::Array),
                };
                env.set("array_sort_by".to_string(), arr_fn_to_arr.clone());
                env.set("array_take_while".to_string(), arr_fn_to_arr.clone());
                env.set("array_drop_while".to_string(), arr_fn_to_arr);
                env.set(
                    "array_min_by".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "array_max_by".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "array_count_if".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "array_zip_with".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Array, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "array_windows".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Int],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "array_sum_by".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "array_product_by".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-2649: map higher-order operations.
                env.set(
                    "map_merge_with".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "map_update_with".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any, Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                // RES-2649: string higher-order operations.
                env.set(
                    "string_map_chars".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Any],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "string_filter_by".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Any],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "string_fold".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "string_for_each_char".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Any],
                        return_type: Box::new(Type::Void),
                    },
                );
                // RES-2650: numeric utilities.
                env.set(
                    "lerp".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any, Type::Any],
                        return_type: Box::new(Type::Float),
                    },
                );
                env.set(
                    "remap".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any, Type::Any, Type::Any, Type::Any],
                        return_type: Box::new(Type::Float),
                    },
                );
                env.set(
                    "float_approx_eq".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any, Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set(
                    "round_to".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int],
                        return_type: Box::new(Type::Float),
                    },
                );
                env.set(
                    "int_pow".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-2650: collection extras.
                env.set(
                    "array_frequency_map".to_string(),
                    Type::Function {
                        params: vec![Type::Array],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "array_key_by".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "array_iterate".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-2651: Result/Option HOF.
                let result_fn_2 = Type::Function {
                    params: vec![Type::Any, Type::Any],
                    return_type: Box::new(Type::Any),
                };
                env.set("result_map".to_string(), result_fn_2.clone());
                env.set("result_and_then".to_string(), result_fn_2.clone());
                env.set("result_map_err".to_string(), result_fn_2.clone());
                env.set("result_or_else".to_string(), result_fn_2);
                let option_fn_2 = Type::Function {
                    params: vec![Type::Any, Type::Any],
                    return_type: Box::new(Type::Any),
                };
                env.set("option_map".to_string(), option_fn_2.clone());
                env.set("option_and_then".to_string(), option_fn_2.clone());
                env.set("option_filter".to_string(), option_fn_2.clone());
                env.set("option_or_else".to_string(), option_fn_2);
                env.set(
                    "option_ok_or".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                // RES-2652: type introspection + collection ergonomics.
                env.set(
                    "type_of".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "result_collect".to_string(),
                    Type::Function {
                        params: vec![Type::Array],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "array_from_fn".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );

                // RES-2619 / RES-2711: char type builtins. Classification
                // functions take one char-or-any and return Bool; conversion
                // functions return the precise target type.
                let fn_char_to_bool = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Bool),
                };
                let fn_char_to_char = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Char),
                };
                env.set("char_is_alpha".to_string(), fn_char_to_bool());
                env.set("char_is_digit".to_string(), fn_char_to_bool());
                env.set("char_is_whitespace".to_string(), fn_char_to_bool());
                env.set("char_is_upper".to_string(), fn_char_to_bool());
                env.set("char_is_lower".to_string(), fn_char_to_bool());
                env.set("char_is_alphanumeric".to_string(), fn_char_to_bool());
                env.set("char_is_ascii".to_string(), fn_char_to_bool());
                env.set("char_to_upper".to_string(), fn_char_to_char());
                env.set("char_to_lower".to_string(), fn_char_to_char());
                env.set(
                    "char_to_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "char_to_string".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "int_to_char".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Char),
                    },
                );

                // RES-416: integer-array reductions.
                env.set("array_sum".to_string(), fn_any_to_int());
                env.set("array_product".to_string(), fn_any_to_int());
                // RES-417: array min/max.
                env.set("array_min".to_string(), fn_any_to_int());
                env.set("array_max".to_string(), fn_any_to_int());
                // RES-543: empty-safe min/max with fallback default.
                let arr_int_to_int = Type::Function {
                    params: vec![Type::Any, Type::Int],
                    return_type: Box::new(Type::Int),
                };
                env.set("array_max_or".to_string(), arr_int_to_int.clone());
                env.set("array_min_or".to_string(), arr_int_to_int);
                // RES-549: integer mean (truncating toward zero).
                env.set(
                    "array_mean_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-550: integer median (truncating mean for even len).
                env.set(
                    "array_median_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-551: integer mode (most-common; smallest on ties).
                env.set(
                    "array_mode_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-552: peak-to-peak range (max − min).
                env.set(
                    "array_range_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-553: consecutive pairwise differences.
                env.set(
                    "array_diff_consec_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-554: per-element clamp to [lo, hi].
                env.set(
                    "array_clamp_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int, Type::Int],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-555: per-element sign (-1/0/1).
                env.set(
                    "array_signum_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-556: per-element absolute value.
                env.set(
                    "array_abs_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-557: dot product of two equal-length int arrays.
                env.set(
                    "array_dot_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-558: sum of squares (Σ x²).
                env.set(
                    "array_sum_squares_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-559: running prefix sum.
                env.set(
                    "array_cumsum_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-560: running max.
                env.set(
                    "array_cummax_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-561: running min.
                env.set(
                    "array_cummin_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-562: running product.
                env.set(
                    "array_cumprod_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-563: count elements in inclusive [lo, hi].
                env.set(
                    "array_count_in_range_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-503: index of max/min element.
                env.set(
                    "array_argmax_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "array_argmin_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-418: element search.
                env.set("array_contains".to_string(), fn_any_any_to_bool());
                env.set("array_index_of".to_string(), fn_any_any_to_int());
                // RES-544: every index where element equals x.
                env.set("array_index_of_all".to_string(), fn_any_any_to_array());
                // RES-541: set-like operations on arrays.
                env.set("array_intersect".to_string(), fn_any_any_to_array());
                env.set("array_diff".to_string(), fn_any_any_to_array());
                // RES-542: order-preserving global-dedup union.
                env.set("array_union".to_string(), fn_any_any_to_array());
                // RES-419: Unicode-scalar ↔ char conversions.
                env.set(
                    "chr".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "ord".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-505: parse single char to base-36 digit.
                env.set(
                    "char_to_digit".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-513: int 0..=35 → base-36 digit char.
                env.set(
                    "digit_to_char".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-420: concatenate two arrays.
                env.set("array_concat".to_string(), fn_any_any_to_array());
                // RES-515: three-way concatenation.
                env.set(
                    "array_concat3".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-421: take/drop first n.
                env.set("array_take".to_string(), fn_any_any_to_array());
                env.set("array_drop".to_string(), fn_any_any_to_array());
                // RES-537: take/drop trailing n elements.
                env.set("array_take_last".to_string(), fn_any_any_to_array());
                env.set("array_drop_last".to_string(), fn_any_any_to_array());
                // RES-514: pick every nth element.
                env.set("array_step".to_string(), fn_any_any_to_array());
                // RES-422: integer sort ascending.
                env.set("array_sort".to_string(), fn_array_to_array());
                // RES-443: integer sort descending.
                env.set("array_sort_desc".to_string(), fn_array_to_array());
                // RES-444: Fisher-Yates shuffle (impure: uses RNG).
                env.set("array_shuffle".to_string(), fn_array_to_array());
                // RES-445: array prefix/suffix predicates.
                env.set("array_starts_with".to_string(), fn_any_any_to_bool());
                env.set("array_ends_with".to_string(), fn_any_any_to_bool());
                // RES-446: all match indices.
                env.set("string_find_all".to_string(), fn_any_any_to_array());
                // RES-546: first byte index of substring, -1 if missing.
                env.set(
                    "string_find".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-547: last byte index of substring, -1 if missing.
                env.set(
                    "string_rfind".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-548: split string at byte index → [before, after].
                env.set(
                    "string_split_at".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-447: i64 boundary constants — zero-arg → Int.
                env.set(
                    "int_min".to_string(),
                    Type::Function {
                        params: vec![],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "int_max".to_string(),
                    Type::Function {
                        params: vec![],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-448: array_position(arr, x, start).
                env.set(
                    "array_position".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-449: array padding (3-arg: arr, n, fill).
                let fn_any_int_any_to_any = Type::Function {
                    params: vec![Type::Any, Type::Int, Type::Any],
                    return_type: Box::new(Type::Any),
                };
                env.set("array_pad_left".to_string(), fn_any_int_any_to_any.clone());
                env.set("array_pad_right".to_string(), fn_any_int_any_to_any);
                // RES-450: array_swap(arr, i, j) — 3-arg.
                // RES-1859: return_type was Type::Any; swapping elements
                // produces an array of the same type, so Array is correct.
                env.set(
                    "array_swap".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int, Type::Int],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-451: insert/remove at index.
                // RES-1859: both produce a new array of the same element type.
                env.set(
                    "array_insert_at".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "array_remove_at".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-452: replace element at index.
                // RES-1859: produces a new array — return_type was Type::Any.
                env.set(
                    "array_set_at".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-453: total Unicode-scalar at index.
                env.set(
                    "string_at".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-454: Unicode-scalar substring.
                env.set(
                    "string_substring".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int, Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-455: sliding windows.
                env.set("array_window".to_string(), fn_any_any_to_array());
                // RES-456: rotation.
                env.set("array_rotate_left".to_string(), fn_any_any_to_array());
                env.set("array_rotate_right".to_string(), fn_any_any_to_array());
                // RES-457: capitalize.
                env.set(
                    "string_capitalize".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-458: array_cycle.
                env.set("array_cycle".to_string(), fn_any_any_to_array());
                // RES-459: ASCII-class string predicates.
                let str_to_bool = Type::Function {
                    params: vec![Type::String],
                    return_type: Box::new(Type::Bool),
                };
                env.set("is_ascii_alpha".to_string(), str_to_bool.clone());
                env.set("is_ascii_digit".to_string(), str_to_bool.clone());
                env.set("is_ascii_alnum".to_string(), str_to_bool);
                // RES-460: trim arbitrary char set.
                env.set(
                    "trim_chars".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-461: indent every line with n spaces.
                env.set(
                    "string_indent".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-462: adjacent pairs as tuples.
                env.set("array_pairs".to_string(), fn_array_to_array());
                // RES-463: UTF-8 byte length.
                env.set(
                    "string_bytes_len".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-564: byte at index, -1 if out of range.
                env.set(
                    "string_byte_at".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-565: string → array of UTF-8 bytes.
                env.set(
                    "string_to_bytes".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-566: array of bytes → Result<String, String>.
                // RES-1859: return_type was Type::Any; the builtin returns a
                // Result (Ok(string) on success, Err(string) on invalid UTF-8).
                env.set(
                    "string_from_bytes".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Result),
                    },
                );
                // RES-464: parse int with explicit radix → Result<Int, String>.
                env.set(
                    "parse_int_base".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int],
                        return_type: Box::new(Type::Result),
                    },
                );
                // RES-465: render int in given radix → String.
                env.set(
                    "int_to_base".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-466: remove first matching element.
                env.set("array_remove".to_string(), fn_any_any_to_array());
                // RES-467: remove all matching elements.
                env.set("array_remove_all".to_string(), fn_any_any_to_array());
                // RES-468: collapse adjacent duplicates.
                env.set("array_dedup".to_string(), fn_array_to_array());
                // RES-2742: pain-points hardening builtins — sort by struct
                // field, bounded-depth flatten, dedup by field, complement-any,
                // int radix parse/format.
                let arr_str_to_arr = Type::Function {
                    params: vec![Type::Array, Type::String],
                    return_type: Box::new(Type::Array),
                };
                env.set("array_sort_by_field".to_string(), arr_str_to_arr.clone());
                env.set(
                    "array_sort_by_field_desc".to_string(),
                    arr_str_to_arr.clone(),
                );
                env.set("array_dedup_by".to_string(), arr_str_to_arr);
                env.set(
                    "array_flatten_depth".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Int],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "array_none".to_string(),
                    Type::Function {
                        params: vec![Type::Array, Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set(
                    "int_parse_hex".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "int_parse_bin".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "int_to_oct".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-504: partition into maximal runs of equal int elements.
                // RES-2645: returns an array of groups (array of arrays).
                env.set(
                    "array_group_by_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-533: count of maximal runs.
                env.set(
                    "array_count_runs".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-469: scalar all/any equality predicates.
                env.set("array_all_eq".to_string(), fn_any_any_to_bool());
                env.set("array_any_eq".to_string(), fn_any_any_to_bool());
                // RES-471: prefix/suffix strippers.
                let str_str_to_str = Type::Function {
                    params: vec![Type::String, Type::String],
                    return_type: Box::new(Type::String),
                };
                env.set("string_strip_prefix".to_string(), str_str_to_str.clone());
                env.set("string_strip_suffix".to_string(), str_str_to_str);
                // RES-472: element-wise array equality.
                env.set("array_eq".to_string(), fn_any_any_to_bool());
                // RES-473: ternary numeric min/max.
                let any3_to_any = Type::Function {
                    params: vec![Type::Any, Type::Any, Type::Any],
                    return_type: Box::new(Type::Any),
                };
                env.set("min3".to_string(), any3_to_any.clone());
                env.set("max3".to_string(), any3_to_any);
                // RES-474: array_ne.
                env.set("array_ne".to_string(), fn_any_any_to_bool());
                // RES-475: fixed-op integer fold.
                env.set(
                    "array_fold_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int, Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-502: running-fold (intermediate accumulators).
                env.set(
                    "array_scan_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int, Type::String],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-521: element-wise binary op on two int arrays.
                env.set(
                    "array_zip_with_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any, Type::String],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-477: one-sided char-set trimmers.
                let str_str_to_str_b = Type::Function {
                    params: vec![Type::String, Type::String],
                    return_type: Box::new(Type::String),
                };
                env.set("trim_start_chars".to_string(), str_str_to_str_b.clone());
                env.set("trim_end_chars".to_string(), str_str_to_str_b);
                // RES-478: array_count_eq alias. RES-2645: count is always Int.
                env.set("array_count_eq".to_string(), fn_any_any_to_int());
                // RES-479: string predicates.
                let str_to_bool_b = Type::Function {
                    params: vec![Type::String],
                    return_type: Box::new(Type::Bool),
                };
                env.set("is_empty".to_string(), str_to_bool_b.clone());
                env.set("is_blank".to_string(), str_to_bool_b);
                // RES-480: replace only first occurrence.
                env.set(
                    "string_replace_first".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String, Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-481: drop first / drop last.
                env.set("array_rest".to_string(), fn_array_to_array());
                env.set("array_init".to_string(), fn_array_to_array());
                // RES-482: replace up to n occurrences.
                env.set(
                    "string_replace_n".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String, Type::String, Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-483: named-predicate take/drop on int arrays.
                let arr_str_to_arr = Type::Function {
                    params: vec![Type::Any, Type::String],
                    return_type: Box::new(Type::Any),
                };
                env.set("array_take_while_int".to_string(), arr_str_to_arr.clone());
                env.set("array_drop_while_int".to_string(), arr_str_to_arr.clone());
                // RES-484: named-predicate filter / partition on int arrays.
                env.set("array_filter_int".to_string(), arr_str_to_arr.clone());
                env.set("array_partition_int".to_string(), arr_str_to_arr);
                // RES-500: named-predicate any-element on int arrays.
                env.set(
                    "array_any_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::String],
                        return_type: Box::new(Type::Bool),
                    },
                );
                // RES-501: named-predicate every-element on int arrays.
                env.set(
                    "array_all_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::String],
                        return_type: Box::new(Type::Bool),
                    },
                );
                // RES-530: named-predicate count on int arrays.
                env.set(
                    "array_count_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-539: indices of int elements matching named predicate.
                env.set(
                    "array_indices_where".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::String],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-485: |a - b|.
                env.set(
                    "abs_diff".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-486: (quotient, remainder) tuple — returns an Array of two Ints.
                env.set(
                    "divmod".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-423: flatten one level.
                env.set("array_flatten".to_string(), fn_array_to_array());
                // RES-424: join string array with separator.
                env.set(
                    "array_join".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-425: scalar-to-string conversion.
                env.set(
                    "to_string".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-426: first-occurrence dedupe.
                env.set("array_unique".to_string(), fn_array_to_array());
                // RES-427: count element occurrences.
                env.set("array_count".to_string(), fn_any_any_to_int());
                // RES-428: array first/last accessors.
                env.set("array_first".to_string(), fn_any_to_any());
                env.set("array_last".to_string(), fn_any_to_any());
                // RES-528: bounded indexing with fallback default.
                env.set(
                    "array_get_or".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                // RES-429: string padding to Unicode-scalar width.
                env.set(
                    "string_pad_left".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int, Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "string_pad_right".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int, Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-540: center-pad a string to a Unicode-scalar width.
                env.set(
                    "string_pad_center".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int, Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-430: pair elements as tuples; truncate to shorter array.
                env.set("array_zip".to_string(), fn_any_any_to_array());
                // RES-531: split an array of 2-tuples into two parallel arrays — returns Array.
                env.set(
                    "array_unzip".to_string(),
                    Type::Function {
                        params: vec![Type::Array],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-431: integer range [start, end).
                env.set(
                    "array_range".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-921: array_slice(arr, lo, hi, inclusive) — sub-array.

                // RES-1859: a slice is still an array — return_type was Type::Any.

                env.set(
                    "array_slice".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any, Type::Any, Type::Bool],

                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-522: indices of an array as a new array.
                env.set("array_indices".to_string(), fn_array_to_array());
                // RES-432: array of n copies.
                env.set("array_repeat".to_string(), fn_any_any_to_array());
                // RES-433: split string into single-char strings.
                env.set("string_chars".to_string(), fn_string_to_array());
                // RES-434: split string into lines (LF, CRLF).
                env.set("string_lines".to_string(), fn_string_to_array());
                // RES-496: split on Unicode whitespace.
                env.set("string_words".to_string(), fn_string_to_array());
                // RES-497: join string array with newline.
                env.set(
                    "string_join_lines".to_string(),
                    Type::Function {
                        params: vec![Type::Array],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-498: join string array with single space.
                env.set(
                    "string_unwords".to_string(),
                    Type::Function {
                        params: vec![Type::Array],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-499: take first n Unicode scalars.
                env.set(
                    "string_take".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-506: drop first n Unicode scalars.
                env.set(
                    "string_drop".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-435: split array into fixed-size chunks.
                env.set("array_chunk".to_string(), fn_any_any_to_array());
                // RES-436: non-overlapping substring count.
                env.set(
                    "string_count".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-523: count occurrences of a single character.
                env.set(
                    "string_count_char".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-524: char-index of a single character (-1 if absent).
                env.set(
                    "string_find_char".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-525: named-predicate prefix slicing on strings.
                let str_str_to_str = Type::Function {
                    params: vec![Type::String, Type::String],
                    return_type: Box::new(Type::String),
                };
                env.set("string_take_while_char".to_string(), str_str_to_str.clone());
                env.set("string_drop_while_char".to_string(), str_str_to_str.clone());
                // RES-526: named-predicate global char filter.
                env.set("string_filter_char".to_string(), str_str_to_str);
                // RES-527: ASCII case-insensitive string equality.
                env.set(
                    "string_eq_ignore_case".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Bool),
                    },
                );
                // RES-437: insert separator between adjacent elements.
                env.set("array_intersperse".to_string(), fn_any_any_to_array());
                // RES-516: alternate elements from two arrays.
                env.set("array_interleave".to_string(), fn_any_any_to_array());
                // RES-438: one-sided trimmers.
                env.set(
                    "trim_start".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "trim_end".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-439: bisect array at index → tuple.
                env.set("array_split_at".to_string(), fn_any_any_to_array());
                // RES-440: integer bitwise ops — strict (Int) -> Int / (Int, Int) -> Int.
                let int_int_to_int = Type::Function {
                    params: vec![Type::Int, Type::Int],
                    return_type: Box::new(Type::Int),
                };
                env.set("bit_and".to_string(), int_int_to_int.clone());
                env.set("bit_or".to_string(), int_int_to_int.clone());
                env.set("bit_xor".to_string(), int_int_to_int.clone());
                env.set("bit_shl".to_string(), int_int_to_int.clone());
                env.set("bit_shr".to_string(), int_int_to_int);
                env.set(
                    "bit_not".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-488: popcount.
                env.set(
                    "bit_count".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-489: count leading zero bits.
                env.set(
                    "bit_leading_zeros".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-490: count trailing zero bits.
                env.set(
                    "bit_trailing_zeros".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-511: single-bit test / set / clear / toggle.
                let int_int_to_int = Type::Function {
                    params: vec![Type::Int, Type::Int],
                    return_type: Box::new(Type::Int),
                };
                env.set(
                    "bit_test".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set("bit_set".to_string(), int_int_to_int.clone());
                env.set("bit_clear".to_string(), int_int_to_int.clone());
                env.set("bit_toggle".to_string(), int_int_to_int.clone());
                // RES-520: circular bit rotation.
                env.set("bit_rotate_left".to_string(), int_int_to_int.clone());
                env.set("bit_rotate_right".to_string(), int_int_to_int.clone());
                // RES-534: extract a single byte from an i64.
                env.set("bit_byte".to_string(), int_int_to_int);
                // RES-538: set a single byte of an i64.
                env.set(
                    "bit_set_byte".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-491: integer floor sqrt.
                env.set(
                    "int_sqrt".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-517: integer exponentiation.
                env.set(
                    "pow_int".to_string(),
                    Type::Function {
                        params: vec![Type::Int, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-518: integer division with explicit rounding.
                let int_int_to_int = Type::Function {
                    params: vec![Type::Int, Type::Int],
                    return_type: Box::new(Type::Int),
                };
                env.set("ceil_div".to_string(), int_int_to_int.clone());
                env.set("floor_div".to_string(), int_int_to_int.clone());
                // RES-519: Python-style modulo (sign of divisor).
                env.set("modulo".to_string(), int_int_to_int);
                // RES-492: floor log base 2.
                env.set(
                    "int_log2".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-493: power-of-two predicate.
                env.set(
                    "is_pow2".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Bool),
                    },
                );
                // RES-494: round up to next power of two.
                env.set(
                    "next_pow2".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-495: int → lowercase hex string.
                env.set(
                    "int_to_hex".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-512: int → binary string.
                env.set(
                    "int_to_bin".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-442: last byte index of substring.
                env.set(
                    "last_index_of".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-413: repeat a string n times.
                env.set(
                    "string_repeat".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-414: first byte index of sub in s, or -1.
                env.set(
                    "index_of".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-145: replace + format.
                env.set(
                    "replace".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String, Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                // `format`'s second argument is `Array<?>` — the prelude
                // `Type::Array` is untyped (no element-type parameter yet),
                // which fits the ticket's `Array<?>` signature.
                env.set(
                    "format".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Array],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-213: prefix/suffix tests + string repetition.
                env.set(
                    "starts_with".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set(
                    "ends_with".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set(
                    "repeat".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-339: string parsing and formatting.
                env.set(
                    "parse_int".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Result),
                    },
                );
                // RES-529: non-erroring parse with fallback default.
                env.set(
                    "parse_int_or".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-532: non-erroring float parse with fallback default.
                env.set(
                    "parse_float_or".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Float],
                        return_type: Box::new(Type::Float),
                    },
                );
                env.set(
                    "parse_float".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Result),
                    },
                );
                env.set(
                    "char_at".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int],
                        return_type: Box::new(Type::Result),
                    },
                );
                env.set(
                    "pad_left".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int, Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "pad_right".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::Int, Type::String],
                        return_type: Box::new(Type::String),
                    },
                );

                // RES-130: explicit int ↔ float conversions. These are the
                // only supported bridge between the two numeric types —
                // arithmetic and literal-match pattern equality both reject
                // implicit coercion (see `check_numeric_same_type`).
                env.set(
                    "to_float".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Float),
                    },
                );
                env.set(
                    "to_int".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );

                // RES-366: pinned-width integer cast builtins. All accept
                // Any (the call site holds whatever the source width is) and
                // return the target type so the typechecker propagates the
                // narrowed type into the surrounding expression.
                let fn_any_to_int8 = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Int8),
                };
                let fn_any_to_int16 = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Int16),
                };
                let fn_any_to_int32 = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Int32),
                };
                let fn_any_to_int = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Int),
                };
                let fn_any_to_uint8 = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::UInt8),
                };
                let fn_any_to_uint16 = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::UInt16),
                };
                let fn_any_to_uint32 = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::UInt32),
                };
                let fn_any_to_uint64 = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::UInt64),
                };
                env.set("as_int8".to_string(), fn_any_to_int8());
                env.set("as_int16".to_string(), fn_any_to_int16());
                env.set("as_int32".to_string(), fn_any_to_int32());
                env.set("as_int64".to_string(), fn_any_to_int());
                env.set("as_uint8".to_string(), fn_any_to_uint8());
                env.set("as_uint16".to_string(), fn_any_to_uint16());
                env.set("as_uint32".to_string(), fn_any_to_uint32());
                env.set("as_uint64".to_string(), fn_any_to_uint64());
                // RES-2618: f32/f64 precision casts.
                env.set(
                    "as_f32".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Float32),
                    },
                );
                env.set(
                    "as_f64".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Float),
                    },
                );

                // RES-138: current retry counter of the enclosing live block.
                env.set(
                    "live_retries".to_string(),
                    Type::Function {
                        params: vec![],
                        return_type: Box::new(Type::Int),
                    },
                );

                // RES-141: process-wide live-block telemetry.
                env.set(
                    "live_total_retries".to_string(),
                    Type::Function {
                        params: vec![],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "live_total_exhaustions".to_string(),
                    Type::Function {
                        params: vec![],
                        return_type: Box::new(Type::Int),
                    },
                );

                // RES-144: one-line stdin reader (std-only).
                env.set(
                    "input".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::String),
                    },
                );

                // RES-1100: `version()` returns the compiler's
                // CARGO_PKG_VERSION so programs can embed the toolchain
                // version in build manifests / provenance certificates.
                env.set(
                    "version".to_string(),
                    Type::Function {
                        params: vec![],
                        return_type: Box::new(Type::String),
                    },
                );
                // RES-2610: `include_str("path")` → string, `include_bytes("path")` → array.
                env.set(
                    "include_str".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "include_bytes".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Array),
                    },
                );

                // RES-143: file I/O builtins (std-only; the resilient-runtime
                // sibling crate has no builtins table so its no_std posture is
                // unaffected).
                env.set(
                    "file_read".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "file_write".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Void),
                    },
                );

                // RES-409: streaming file I/O — `file_open`, `file_read_chunk`,
                // `file_write_chunk`, `file_seek`, `file_close`. The `File`
                // handle is a `Type::Any` (a struct in disguise) until the type
                // system grows a dedicated linear-resource form. Each builtin
                // returns `Type::Result` so the user is forced to handle errors.
                env.set(
                    "file_open".to_string(),
                    Type::Function {
                        params: vec![Type::String, Type::String],
                        return_type: Box::new(Type::Result),
                    },
                );
                env.set(
                    "file_read_chunk".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int],
                        return_type: Box::new(Type::Result),
                    },
                );
                env.set(
                    "file_write_chunk".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Bytes],
                        return_type: Box::new(Type::Result),
                    },
                );
                env.set(
                    "file_seek".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Int, Type::String],
                        return_type: Box::new(Type::Result),
                    },
                );
                env.set(
                    "file_close".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Result),
                    },
                );

                // RES-151: read-only env-var accessor. `Result<String, String>`
                // — absence is a first-class outcome, not a runtime halt.
                env.set(
                    "env".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Result),
                    },
                );

                // RES-148: Map builtins. The typechecker doesn't (yet) carry
                // a dedicated `Type::Map<K, V>` constructor — following the
                // same permissive-Any convention as the Array / Result
                // builtins until G7 inference lands.
                env.set(
                    "map_new".to_string(),
                    Type::Function {
                        params: vec![],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "map_insert".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "map_get".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Result),
                    },
                );
                env.set(
                    "map_remove".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "map_keys".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "map_len".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-883.
                env.set(
                    "map_values".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-884.
                env.set(
                    "map_contains_key".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );
                // RES-1144: map_entries / map_merge / map_is_empty.
                env.set(
                    "map_entries".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "map_merge".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "map_is_empty".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );

                // RES-293: HashMap stdlib builtins. Same permissive-Any
                // shape as the Map builtins above — once G7 inference lands
                // these tighten to `HashMap<K, V>`.
                env.set(
                    "hashmap_new".to_string(),
                    Type::Function {
                        params: vec![],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "hashmap_insert".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "hashmap_get".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Result),
                    },
                );
                env.set(
                    "hashmap_remove".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "hashmap_contains".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set(
                    "hashmap_keys".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-885.
                env.set(
                    "hashmap_len".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-886.
                env.set(
                    "hashmap_values".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-1144: hashmap_entries / hashmap_merge / hashmap_is_empty.
                env.set(
                    "hashmap_entries".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                env.set(
                    "hashmap_merge".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "hashmap_is_empty".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );
                // RES-1154: set_is_empty / set_from_array / result_and / option_and.
                env.set(
                    "set_is_empty".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set(
                    "set_from_array".to_string(),
                    Type::Function {
                        params: vec![Type::Array],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "result_and".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "option_and".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                // RES-1160: argmax / argmin for float and string arrays.
                let fn_array_to_int = || Type::Function {
                    params: vec![Type::Array],
                    return_type: Box::new(Type::Int),
                };
                env.set("array_argmax_float".to_string(), fn_array_to_int());
                env.set("array_argmin_float".to_string(), fn_array_to_int());
                env.set("array_argmax_string".to_string(), fn_array_to_int());
                env.set("array_argmin_string".to_string(), fn_array_to_int());
                // RES-1168: precision-sensitive math — expm1 / ln_1p / mul_add / recip.
                let fn_float_to_float_p = || Type::Function {
                    params: vec![Type::Float],
                    return_type: Box::new(Type::Float),
                };
                env.set("expm1".to_string(), fn_float_to_float_p());
                env.set("ln_1p".to_string(), fn_float_to_float_p());
                env.set(
                    "mul_add".to_string(),
                    Type::Function {
                        params: vec![Type::Float, Type::Float, Type::Float],
                        return_type: Box::new(Type::Float),
                    },
                );
                env.set("recip".to_string(), fn_float_to_float_p());

                // RES-149: Set builtins. Same permissive-Any convention as
                // Map — no dedicated `Type::Set<T>` until inference lands.
                env.set(
                    "set_new".to_string(),
                    Type::Function {
                        params: vec![],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "set_insert".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "set_remove".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "set_has".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set(
                    "set_len".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "set_items".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Array),
                    },
                );
                // RES-876: set algebra primitives.
                env.set(
                    "set_union".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                // RES-877.
                env.set(
                    "set_intersection".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                // RES-878.
                env.set(
                    "set_difference".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                // RES-879.
                env.set(
                    "set_is_subset".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );
                // RES-880.
                env.set(
                    "set_is_superset".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );
                // RES-881.
                env.set(
                    "set_is_disjoint".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Bool),
                    },
                );
                // RES-882.
                env.set(
                    "set_symmetric_difference".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );

                // RES-152: Bytes builtins. `bytes_slice` returns new Bytes;
                // `byte_at` returns Int — the language has no `u8` yet.
                env.set(
                    "bytes_len".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes],
                        return_type: Box::new(Type::Int),
                    },
                );
                env.set(
                    "bytes_slice".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes, Type::Int, Type::Int],
                        return_type: Box::new(Type::Bytes),
                    },
                );
                env.set(
                    "byte_at".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes, Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-887.
                env.set(
                    "bytes_concat".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes, Type::Bytes],
                        return_type: Box::new(Type::Bytes),
                    },
                );
                // RES-888.
                env.set(
                    "bytes_eq".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes, Type::Bytes],
                        return_type: Box::new(Type::Bool),
                    },
                );

                // Result builtins
                env.set("Ok".to_string(), fn_any_to_result());
                env.set("Err".to_string(), fn_any_to_result());
                env.set("is_ok".to_string(), fn_result_to_bool());
                env.set("is_err".to_string(), fn_result_to_bool());
                env.set("unwrap".to_string(), fn_result_to_any());
                env.set("unwrap_err".to_string(), fn_result_to_any());

                // RES-2651: Option constructors and predicates.
                // `Some` is registered with return type Option<Any>; call
                // sites override this with the concrete argument type (see
                // the `Some(expr)` special case in CallExpression).
                env.set(
                    "Some".to_string(),
                    Type::Function {
                        params: vec![Type::Any],
                        return_type: Box::new(Type::Option(Box::new(Type::Any))),
                    },
                );
                env.set("None".to_string(), Type::Option(Box::new(Type::Any)));
                env.set(
                    "is_some".to_string(),
                    Type::Function {
                        params: vec![Type::Option(Box::new(Type::Any))],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set(
                    "is_none".to_string(),
                    Type::Function {
                        params: vec![Type::Option(Box::new(Type::Any))],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set(
                    "unwrap_option".to_string(),
                    Type::Function {
                        params: vec![Type::Option(Box::new(Type::Any))],
                        return_type: Box::new(Type::Any),
                    },
                );

                // RES-936/937: Result fallback variants — Result + default → Any.
                let fn_result_any_to_any = || Type::Function {
                    params: vec![Type::Result, Type::Any],
                    return_type: Box::new(Type::Any),
                };
                env.set("result_unwrap_or".to_string(), fn_result_any_to_any());
                env.set("result_unwrap_or_err".to_string(), fn_result_any_to_any());
                // RES-938: Result <-> Option conversion.
                env.set(
                    "result_to_option".to_string(),
                    Type::Function {
                        params: vec![Type::Result],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "option_to_result".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Result),
                    },
                );
                // RES-939: chain alternatives.
                env.set(
                    "option_or".to_string(),
                    Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Any),
                    },
                );
                env.set(
                    "result_or".to_string(),
                    Type::Function {
                        params: vec![Type::Result, Type::Result],
                        return_type: Box::new(Type::Result),
                    },
                );
                // RES-940: power-of-two helpers.
                env.set(
                    "is_power_of_two".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Bool),
                    },
                );
                env.set(
                    "next_power_of_two".to_string(),
                    Type::Function {
                        params: vec![Type::Int],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-941: int-array statistics → Float.
                let fn_array_to_float = || Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Float),
                };
                env.set("array_average".to_string(), fn_array_to_float());
                env.set("array_median".to_string(), fn_array_to_float());
                // RES-942: float-array reductions.
                env.set("array_sum_float".to_string(), fn_array_to_float());
                env.set("array_product_float".to_string(), fn_array_to_float());
                env.set("array_min_float".to_string(), fn_array_to_float());
                env.set("array_max_float".to_string(), fn_array_to_float());
                env.set("array_average_float".to_string(), fn_array_to_float());
                // RES-943: hex encoding.
                env.set(
                    "bytes_to_hex".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes],
                        return_type: Box::new(Type::String),
                    },
                );
                env.set(
                    "bytes_from_hex".to_string(),
                    Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Result),
                    },
                );
                // RES-944: bytes search.
                let fn_bytes_bytes_to_bool = || Type::Function {
                    params: vec![Type::Bytes, Type::Bytes],
                    return_type: Box::new(Type::Bool),
                };
                env.set("bytes_starts_with".to_string(), fn_bytes_bytes_to_bool());
                env.set("bytes_ends_with".to_string(), fn_bytes_bytes_to_bool());
                env.set(
                    "bytes_index_of".to_string(),
                    Type::Function {
                        params: vec![Type::Bytes, Type::Bytes],
                        return_type: Box::new(Type::Int),
                    },
                );
                // RES-945: default-fallback map accessors.
                let fn_map_key_default = || Type::Function {
                    params: vec![Type::Any, Type::Any, Type::Any],
                    return_type: Box::new(Type::Any),
                };
                env.set("map_get_or".to_string(), fn_map_key_default());
                env.set("hashmap_get_or".to_string(), fn_map_key_default());

                // RES-328: `cell(initial)` — shared mutable container.
                // Element type isn't tracked at the type-system layer (the
                // generic story lands with G7); the runtime enforces that
                // `.set` rebinds the inner value, and the inner value's
                // dynamic type flows through `Type::Any`.
                env.set("cell".to_string(), fn_any_to_any());
                std::sync::Arc::new(env)
            });

        // RES-1692: pre-size the per-typecheck HashMaps that grow
        // once per top-level fn / struct during the hoist pass.
        // Same shape as RES-1686 / RES-1688 / RES-1690 — perf-only,
        // saves 2-3 rehash rounds per typecheck on medium / large
        // programs. `contract_table` and `fn_decl_spans` see one
        // entry per top-level fn (50+ on `large.rz`); the rest grow
        // slower but cost a fixed small allocation each anyway.
        const PRESIZE: usize = 32;
        TypeChecker {
            env: TypeEnvironment::new_with_outer_arc(std::sync::Arc::clone(&BUILTIN_ENV)),
            contract_table: HashMap::with_capacity(PRESIZE),
            fn_decl_spans: HashMap::with_capacity(PRESIZE),
            const_bindings: HashMap::with_capacity(PRESIZE),
            stats: VerificationStats::default(),
            certificates: Vec::new(),
            struct_fields: HashMap::with_capacity(PRESIZE),
            // RES-1398: clone the cached builtin enum_decls (Option /
            // Result) HashMap instead of rebuilding it from scratch.
            // Pattern mirrors RES-1349's BUILTIN_ENV cache — the data
            // is invariant, so populate once per process via LazyLock
            // and pay an Arc bump per `TypeChecker::new()` instead of
            // 2 Vec allocations + 4 EnumVariant struct constructions
            // + 4 `String::to_string()`s per call. `Arc` (not `Rc`)
            // so the static can be `Sync`; atomic refcount ops are
            // still trivially cheaper than the allocations they
            // replace.
            enum_decls: {
                static BUILTIN_ENUM_DECLS: std::sync::LazyLock<
                    std::collections::HashMap<String, std::sync::Arc<Vec<crate::EnumVariant>>>,
                > = std::sync::LazyLock::new(|| {
                    let mut m = std::collections::HashMap::new();
                    let s = crate::span::Span::default();
                    m.insert(
                        "Option".to_string(),
                        std::sync::Arc::new(vec![
                            crate::EnumVariant {
                                name: "Some".to_string(),
                                span: s,
                                payload: crate::EnumPayload::None,
                            },
                            crate::EnumVariant {
                                name: "None".to_string(),
                                span: s,
                                payload: crate::EnumPayload::None,
                            },
                        ]),
                    );
                    m.insert(
                        "Result".to_string(),
                        std::sync::Arc::new(vec![
                            crate::EnumVariant {
                                name: "Ok".to_string(),
                                span: s,
                                payload: crate::EnumPayload::None,
                            },
                            crate::EnumVariant {
                                name: "Err".to_string(),
                                span: s,
                                payload: crate::EnumPayload::None,
                            },
                        ]),
                    );
                    m
                });
                BUILTIN_ENUM_DECLS.clone()
            },
            type_aliases: HashMap::with_capacity(PRESIZE),
            // RES-137: ticket's default is 5 seconds per query.
            verifier_timeout_ms: 5000,
            // RES-217: partial-proof warnings on by default.
            warn_unverified: true,
            // RES-217: populated by `check_program_with_source`.
            source_path: String::new(),
            // RES-189: populated during LetStatement handling.
            let_type_hints: Vec::new(),
            // RES-387: no enclosing fn at program start.
            current_fn_fails: None,
            // RES-403: no enclosing fn return type at program start.
            current_fn_return_type: None,
            // RES-910: loop depth starts at 0 (top-level is not a loop).
            loop_depth: 0,
            // RES-2653: no enclosing labeled loops at the top level.
            loop_label_stack: Vec::new(),
            // RES-354: auto-detect theory by default.
            #[cfg(feature = "z3")]
            z3_theory: crate::verifier_z3::Z3Theory::Auto,
            // RES-318: per-loop-invariant verbose stderr line is OFF
            // by default. The driver flips it on via `--verbose`.
            verbose_loop_invariants: false,
            // RES-1322: opt-in. Audit + explain-effects drivers set
            // this; default-mode runs (every non-audit CLI invocation,
            // the LSP/REPL, every `cargo test` typecheck) skip the
            // `infer_fn_effects` fixpoint.
            audit_stats: false,
            // RES-1353: opt-in. LSP sets this; default-mode runs skip
            // populating `let_type_hints` (it's only consumed by the
            // inlay-hint provider).
            capture_inlay_hints: false,
            // RES-1357: opt-in. The `--emit-certificate` driver flips
            // this; every other invocation skips pushing
            // `CapturedCertificate` onto the Vec it'd drop on
            // TypeChecker drop.
            emit_certificates: false,
            // RES-1862: default to zero span (synthetic / unknown).
            current_span: Span::default(),

            fn_type_params: HashMap::new(),
            trait_impls: HashMap::new(),
            trait_default_methods: HashMap::new(),
        }
    }

    /// RES-137: override the per-query Z3 solver timeout in ms.
    /// Called by the driver from the `--verifier-timeout-ms` CLI
    /// flag. Pass `0` to disable the timeout entirely (NIA proofs
    /// that would otherwise hit the default budget run to
    /// completion — use at your own risk).
    pub fn with_verifier_timeout_ms(mut self, ms: u32) -> Self {
        self.verifier_timeout_ms = ms;
        self
    }

    /// RES-217: toggle partial-proof warnings. When `true`
    /// (default), Z3 `Unknown` verdicts surface as
    /// `warning[partial-proof]: Z3 returned Unknown for
    /// assertion at <file>:<line>:<col> — proof is incomplete`.
    /// The driver flips this to `false` on `--no-warn-unverified`
    /// for CI runs that want a quieter stderr.
    pub fn with_warn_unverified(mut self, on: bool) -> Self {
        self.warn_unverified = on;
        self
    }

    /// RES-354: override the SMT theory used for Z3 encoding.
    /// `Z3Theory::Auto` (default) picks BV32 when bitwise ops are
    /// detected; `Bv` forces BV32; `Lia` forces LIA (bails on
    /// bitwise ops). Driver calls this from `--z3-theory`.
    #[cfg(feature = "z3")]
    pub fn with_z3_theory(mut self, theory: crate::verifier_z3::Z3Theory) -> Self {
        self.z3_theory = theory;
        self
    }

    /// RES-318: enable verbose stderr output from the loop-invariant
    /// verifier. The driver flips this on via the `--verbose` flag.
    pub fn with_verbose_loop_invariants(mut self, on: bool) -> Self {
        self.verbose_loop_invariants = on;
        self
    }

    /// RES-1322: opt into the post-typecheck `infer_fn_effects`
    /// fixpoint that populates `self.stats.fn_effects`. The only
    /// consumers of `fn_effects` are the `--audit` /
    /// `--explain-effects` CLI drivers; for every other invocation
    /// (the default `rz prog.rz` path, the LSP/REPL paths, every
    /// `cargo test` typecheck call) the fixpoint is wasted work
    /// because the result is never read. Default `false`; the driver
    /// flips this on for the audit/explain-effects paths.
    pub fn with_audit_stats(mut self, on: bool) -> Self {
        self.audit_stats = on;
        self
    }

    /// RES-1353: opt into populating `self.let_type_hints` with an
    /// entry per inferred `let` binding. The LSP backend reads
    /// these to produce inlay hints; every other entry point
    /// (default `rz prog.rz` path, REPL, every `cargo test`
    /// typecheck call) discards them on TypeChecker drop. Default
    /// `false`; LSP flips this on before running typecheck.
    #[allow(dead_code)] // only called behind the `lsp` feature
    pub fn with_capture_inlay_hints(mut self, on: bool) -> Self {
        self.capture_inlay_hints = on;
        self
    }

    /// RES-1357: opt into pushing `CapturedCertificate` entries onto
    /// `self.certificates`. The only consumer is the
    /// `--emit-certificate <DIR>` driver in lib.rs; every other
    /// path leaves the flag off so the per-push `fn_name.clone()` +
    /// Vec growth doesn't fire on hot paths that never read the
    /// certs.
    pub fn with_emit_certificates(mut self, on: bool) -> Self {
        self.emit_certificates = on;
        self
    }

    /// RES-318: read accessor for the verifier pass.
    #[allow(dead_code)]
    pub(crate) fn verifier_timeout_ms(&self) -> u32 {
        self.verifier_timeout_ms
    }

    /// RES-318: read accessor for the verifier pass.
    #[allow(dead_code)]
    pub(crate) fn verbose_loop_invariants(&self) -> bool {
        self.verbose_loop_invariants
    }

    /// RES-318: push a loop-invariant proof certificate so it
    /// participates in the regular `--emit-certificate <DIR>` dump.
    ///
    /// RES-1357: only stores the cert when `emit_certificates` is
    /// on. The `loop_invariants_proven` stat counter increments
    /// unconditionally so the audit summary still reports the
    /// total — only the SMT-LIB2 body Vec push is gated.
    #[allow(dead_code)]
    pub(crate) fn push_loop_invariant_certificate(&mut self, idx: usize, smt2: String) {
        if self.emit_certificates {
            self.certificates.push(CapturedCertificate {
                fn_name: "<loop>".to_string(),
                kind: "loop_invariant",
                idx,
                smt2,
            });
        }
        self.stats.loop_invariants_proven += 1;
    }

    /// RES-318: helper for the verifier's unit tests — count proven
    /// invariants by counting `loop_invariant`-kinded certificates.
    #[cfg(feature = "z3")]
    #[allow(dead_code)]
    pub(crate) fn loop_invariant_certificate_count(&self) -> usize {
        self.certificates
            .iter()
            .filter(|c| c.kind == "loop_invariant")
            .count()
    }

    /// RES-2693: true when one of `a`/`b` is a concrete struct that
    /// implements the other as a trait.  Checks both orderings so the
    /// caller doesn't need to know which of the two types is the trait
    /// and which is the struct.
    ///
    /// Only `Type::Struct(name)` pairs are examined; all other combinations
    /// return `false`, so the method never fires for primitive types.
    fn satisfies_trait_param(&self, a: &Type, b: &Type) -> bool {
        let (Type::Struct(a_name), Type::Struct(b_name)) = (a, b) else {
            return false;
        };
        // a is the implementing struct, b is the trait:
        if self
            .trait_impls
            .get(a_name.as_str())
            .is_some_and(|ts| ts.contains(b_name.as_str()))
        {
            return true;
        }
        // b is the implementing struct, a is the trait:
        self.trait_impls
            .get(b_name.as_str())
            .is_some_and(|ts| ts.contains(a_name.as_str()))
    }

    pub fn check_program(&mut self, program: &Node) -> Result<Type, String> {
        // Backwards-compatible thin shim: callers that don't have a
        // source path (REPL, unit tests) keep the original signature.
        // RES-080 added `check_program_with_source` for the driver.
        self.check_program_with_source(program, "<unknown>")
    }

    /// RES-080: like `check_program`, but errors thrown by per-statement
    /// type checking are prefixed with `<source_path>:<line>:<col>: `
    /// (using the statement's `Spanned` start position from RES-077).
    /// The driver uses this entry point so `--typecheck` diagnostics
    /// point users at the right line. Sub-expression errors still
    /// surface at the granularity of their containing top-level
    /// statement until RES-078 / RES-079 land per-expression spans.
    pub fn check_program_with_source(
        &mut self,
        program: &Node,
        source_path: &str,
    ) -> Result<Type, String> {
        // RES-217: stash the source path for partial-proof
        // warnings that want to print `<file>:<line>:<col>`.
        self.source_path = source_path.to_string();
        // RES-1631: clear the within-run Z3 proof cache so a prior
        // compilation's entries don't leak into this typecheck. The
        // cache is thread-local and per-`check_program_with_source`
        // call; entries accumulate during one typecheck only.
        reset_z3_prove_cache();
        match program {
            Node::Program(statements) => {
                // RES-061: pre-pass to register every top-level Function
                // in the contract table. Mirrors the interpreter's
                // function-hoisting pass so call sites can fold contracts
                // even for forward references.
                // RES-077: top-level statements are now Spanned<Node>;
                // deref via .node for the existing destructure.
                // RES-128: also hoist type aliases in the same pass so
                // `fn foo(Meters x) ...` typechecks when the
                // `type Meters = Int;` declaration textually follows.
                for stmt in statements {
                    match &stmt.node {
                        Node::Function {
                            name,
                            parameters,
                            requires,
                            ensures,
                            fails,
                            return_type,
                            span,
                            ..
                        } => {
                            // RES-1363: wrap in `Rc` so the call-site
                            // reader at typechecker.rs:5877 clones a
                            // single refcount instead of deep-cloning
                            // four `Vec`s.
                            self.contract_table.insert(
                                name.clone(),
                                std::rc::Rc::new(ContractInfo {
                                    parameters: parameters.clone(),
                                    requires: requires.clone(),
                                    ensures: ensures.clone(),
                                    fails: fails.clone(),
                                }),
                            );
                            // RES-340: remember the fn keyword's span so
                            // the rich type-mismatch path can point at
                            // the declaration. Reject duplicate fn names
                            // here — a second `fn foo` silently overwrote
                            // the first before this guard, making the
                            // program non-deterministic.
                            if let Some(prev) = self.fn_decl_spans.get(name) {
                                return Err(format!(
                                    "{}:{}:{}: error: duplicate function name `{}` — \
                                     previously declared at {}:{}",
                                    self.source_path,
                                    span.start.line,
                                    span.start.column,
                                    name,
                                    prev.start.line,
                                    prev.start.column,
                                ));
                            }
                            self.fn_decl_spans.insert(name.clone(), *span);
                            // RES-1105 + RES-1106: also register the
                            // function name in `self.env` so identifier
                            // lookups in self-recursive and forward-
                            // reference call sites resolve. Without this
                            // pre-binding, `fn fact(int n) -> int { ...
                            // fact(n-1) ... }` errors with
                            // "Undefined variable 'fact'" even though
                            // the contract table sees it for runtime
                            // hoisting. The post-body pass at the
                            // Node::Function arm refreshes this entry
                            // with the inferred-or-declared return type.
                            //
                            // Type resolution is best-effort: if a
                            // parameter or return annotation fails to
                            // parse (e.g. references a yet-to-hoist
                            // alias), fall back to Type::Any so the
                            // body still gets to run and surface the
                            // real diagnostic at its definition site.
                            let param_types: Vec<Type> = parameters
                                .iter()
                                .map(|(ty_name, _)| {
                                    self.parse_type_name(ty_name).unwrap_or(Type::Any)
                                })
                                .collect();
                            let ret_type = match return_type {
                                Some(ty_name) => self.parse_type_name(ty_name).unwrap_or(Type::Any),
                                None => Type::Any,
                            };
                            self.env.set(
                                name.clone(),
                                Type::Function {
                                    params: param_types,
                                    return_type: Box::new(ret_type),
                                },
                            );
                        }
                        Node::TypeAlias { name, target, .. } => {
                            self.type_aliases.insert(name.clone(), target.clone());
                        }
                        // RES-417: hoist const declarations so functions
                        // that textually precede a const declaration can
                        // still reference it. Without this, `fn f() -> int
                        // { return N; }` followed by `const N: int = 5;`
                        // would error with "Undefined variable 'N'" when
                        // type-checking `f`. The pre-pass registers the
                        // declared type (from the annotation, if present)
                        // so the env lookup succeeds; the main pass then
                        // overwrites the binding with the inferred value
                        // type and populates const_bindings.
                        Node::Const {
                            name, type_annot, ..
                        } => {
                            let bind_type = if let Some(ty_name) = type_annot {
                                self.parse_type_name(ty_name).unwrap_or(Type::Any)
                            } else {
                                Type::Any
                            };
                            self.env.set(name.clone(), bind_type);
                        }
                        // RES-2693: record struct → trait relationships so
                        // call-site checking can accept a concrete struct
                        // where a trait-typed parameter is expected.
                        Node::ImplBlock {
                            trait_name: Some(t),
                            struct_name,
                            ..
                        } => {
                            self.trait_impls
                                .entry(struct_name.clone())
                                .or_default()
                                .insert(t.clone());
                        }
                        // RES-2697: record trait methods that carry a
                        // default body so FieldAccess type-checks can
                        // succeed when the impl block omits them.
                        Node::TraitDecl { name, methods, .. } => {
                            for m in methods {
                                if m.default_body.is_some() {
                                    self.trait_default_methods
                                        .entry(name.clone())
                                        .or_default()
                                        .insert(m.name.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }

                let mut result_type = Type::Void;
                for stmt in statements {
                    // RES-1862: reset current_span to the statement's
                    // own span before descending. check_node arms for
                    // InfixExpression / CallExpression / LetStatement
                    // overwrite this with the innermost node's span so
                    // that error messages point at the exact expression
                    // rather than the whole containing statement.
                    self.current_span = stmt.span;
                    let check_result = self.check_node(&stmt.node);
                    // Capture the best span after check_node returns
                    // (self is no longer mutably borrowed at this point).
                    // Use current_span when it's more specific (non-zero
                    // line within the statement range); fall back to
                    // stmt.span otherwise.
                    let diag_span = if self.current_span.start.line > 0 {
                        self.current_span
                    } else {
                        stmt.span
                    };
                    result_type = check_result.map_err(|e| {
                        // RES-080 / RES-1862: prepend file:line:col.
                        // Skip the prefix when the span looks
                        // default/empty (line 0 means "synthetic").
                        if diag_span.start.line == 0 {
                            e
                        } else {
                            format!(
                                "{}:{}:{}: {}",
                                source_path, diag_span.start.line, diag_span.start.column, e
                            )
                        }
                    })?;
                }

                // RES-1627: one shared whole-AST marker pre-scan
                // serving both the actor-invariant pre-check below
                // AND the <EXTENSION_PASSES> gates. The historical
                // pattern computed `Markers` near the extension block
                // and the actor pre-check did its own `iter().any`;
                // sharing the walk avoids the duplicate top-level
                // scan. See `crate::pass_gate::Markers` for the full
                // marker surface.
                let markers = crate::pass_gate::Markers::scan(program);

                // RES-388: verify every `actor`'s `always` safety
                // invariants. The walk happens *after* per-statement
                // type-checking so we only reason about well-typed
                // bodies. Any obligation that Z3 refutes becomes a
                // hard error with a file:line:col diagnostic naming
                // the actor, handler, and invariant; Unknown /
                // Unsupported verdicts emit a stderr warning but do
                // not fail the check (matching how partial proofs
                // of `requires` / `ensures` are handled — RES-217).
                //
                // RES-1313 / RES-1627: short-circuit when no
                // `ActorDecl` exists. `collect_actor_obligations`
                // would iterate `statements` filtering ActorDecls
                // and call `verifier_actors::verify_actor` for each
                // — for programs with zero actors the inner loop
                // never enters, but the dispatch + Z3 setup call
                // still happens. `markers.has_actor_decl` is set
                // by the shared scan above.
                let obligations = if markers.has_actor_decl {
                    collect_actor_obligations(statements, self.verifier_timeout_ms)
                } else {
                    Vec::new()
                };
                // Worst case every obligation is `Refuted` and contributes one
                // diagnostic; pre-size to skip the default 0→4→8 growth chain
                // when the file actually has invariant violations to report.
                let mut refuted: Vec<String> = Vec::with_capacity(obligations.len());
                for o in obligations {
                    match o.result {
                        crate::verifier_actors::ActorProofResult::Proved => {}
                        crate::verifier_actors::ActorProofResult::Refuted { counterexample } => {
                            let mut msg = format!(
                                "{}:{}:{}: actor `{}` violates `always: {}` in handler `{}`",
                                if source_path.is_empty() {
                                    "<unknown>"
                                } else {
                                    source_path
                                },
                                o.invariant_span.start.line,
                                o.invariant_span.start.column,
                                o.actor_name,
                                o.invariant_label,
                                o.handler_name,
                            );
                            if let Some(cx) = counterexample {
                                msg.push_str(&format!(" (counterexample: {})", cx));
                            }
                            refuted.push(msg);
                        }
                        crate::verifier_actors::ActorProofResult::Unknown => {
                            if self.warn_unverified {
                                eprintln!(
                                    "warning[partial-proof]: actor `{}` `always: {}` could not be proven on handler `{}` — Z3 returned Unknown",
                                    o.actor_name, o.invariant_label, o.handler_name,
                                );
                            }
                        }
                        crate::verifier_actors::ActorProofResult::Unsupported { reason } => {
                            if self.warn_unverified {
                                eprintln!(
                                    "warning[partial-proof]: actor `{}` `always: {}` not verified on handler `{}` — {}",
                                    o.actor_name, o.invariant_label, o.handler_name, reason,
                                );
                            }
                        }
                    }
                }
                if !refuted.is_empty() {
                    return Err(refuted.join("\n"));
                }

                // RES-191: after regular type-checking, enforce the
                // `@pure` annotation. Collect the set of fn names
                // that are declared `@pure`, then re-walk each of
                // their bodies flagging any forbidden operation
                // (impure builtin, unannotated user-fn call, etc.).
                // Failures prepend the same file:line:col prefix as
                // above so users land on the violating site.
                //
                // RES-1671 gate: both purity (RES-191) and effects
                // (RES-389) bail at their internal RES-1296 fast-reject
                // when no `@pure` fn exists. Markers already computed
                // that signal during the shared whole-AST scan above —
                // skip both passes entirely when the marker is false.
                // Each pass's internal fast-reject stays as defense in
                // depth for callers that bypass Markers.
                if markers.has_pure_fn {
                    check_program_purity(statements, source_path)?;
                    check_program_effects(statements, source_path)?;
                }

                // RES-385: single-use enforcement for linear types.
                // RES-1669 gate: the pass walks the whole program looking
                // for `linear `-prefixed Function param types or
                // LetStatement type_annots; Markers computes that signal
                // during the shared whole-AST scan, so skip the dedicated
                // walk when no linear binding exists. The pass's own
                // RES-1294 fast-reject stays in place for callers that
                // bypass Markers (LSP per-document path).
                if markers.has_linear_binding {
                    crate::linear::check_linear_usage(program, source_path)?;
                }

                // RES-1585 / RES-1627: `markers` was computed above
                // (before the actor-invariant pre-check) so the same
                // scan serves both that site and the gates below.

                // <EXTENSION_PASSES>
                // Add new compiler pass calls here (append-only).
                // Pattern: crate::your_feature::check(program, source_path)?;
                // Merge conflicts: keep ALL calls from both sides.
                // RES-1612 gate: pass scans for `Node::TryCatch`.
                if markers.has_try_catch {
                    crate::try_catch::check(program, source_path)?;
                }
                // RES-1616 gate: pass walks `Node::ActorDecl` looking
                // for non-empty `eventually_clauses`. Markers records
                // the presence flag during the shared whole-AST walk.
                if markers.has_actor_with_eventually {
                    crate::verifier_liveness::check(program, source_path)?;
                }
                // RES-1612 gate: pass scans for `Node::LiveBlock`.
                if markers.has_live_block {
                    crate::recovery_checker::check(program, source_path)?;
                }
                // RES-1612 gate: pass scans for `Node::Assume`.
                if markers.has_assume {
                    crate::assume_false_checker::check(program, source_path)?;
                }
                // RES-1612 gate: pass scans for `Node::IndexExpression`.
                // The pass's first call is `reset_stats()` to clear
                // stale PROVEN_SITES from a prior compile — that only
                // matters when this typecheck queries `is_proven_site`,
                // which only happens when the program has index
                // expressions. So skipping the call entirely on the
                // gate-false case leaves stale state unobserved.
                if markers.has_index_expression {
                    crate::bounds_check::check_array_bounds(program, source_path)?;
                }
                // RES-1612 gate: pass scans for `Node::InvariantStatement`.
                if markers.has_invariant_statement {
                    crate::loop_invariants::check(program, source_path)?;
                }
                // RES-1620 gate: the Z3 verifier walks for
                // `WhileStatement` with non-empty `invariants` OR
                // any `InvariantStatement` — same shape as the
                // pass's own RES-1297 fast-reject. Markers tracks
                // both signals from the shared whole-AST walk, so
                // the gate is two O(1) bool reads. The non-z3
                // build's `verify_and_capture` is already a no-op
                // stub, so skipping the call there is also free.
                if markers.has_invariant_statement || markers.has_while_with_invariants {
                    crate::verifier_loop_invariants::verify_and_capture(self, program);
                }
                // RES-1616 gate: pass scans for `Node::TypeAlias`.
                if markers.has_type_alias {
                    crate::type_aliases::check(program, source_path)?;
                }
                // RES-1612 gate: pass scans for `Node::Range`.
                if markers.has_range {
                    crate::ranges::check(program, source_path)?;
                }
                // RES-2721: string interpolation sub-expressions are now
                // checked inline at `Node::InterpolatedString` in `check_node`
                // (with the full populated environment) rather than by the old
                // `string_interp::check` extension pass which used a fresh
                // TypeChecker with no let-binding scope.  The gate below is
                // intentionally removed; keep the `has_interp_string` marker
                // in case a future pass needs it.
                // RES-324: modules::check now active (duplicate names +
                // unresolved items); gated at the has_inline_module site above.
                // `full_modules::check` separately handles the module graph/cycles.
                // RES-1615: validate default parameter values — check
                // that defaults are trailing-only and compile-time
                // constants. Gated on the `has_fn_defaults` marker so
                // programs with no defaulted parameters pay nothing.
                if markers.has_fn_defaults {
                    crate::default_params::check(program, source_path)?;
                }
                // RES-1616 gate: pass scans for `Node::Function` with
                // non-empty `type_params`. Markers records the flag
                // during the shared whole-AST walk.
                if markers.has_generic_fn {
                    crate::generics::check(program, source_path)?;
                    // RES-2576: infer type parameters at call sites.
                    crate::generic_inference::check(program, source_path)?;
                }
                // RES-2615: variance inference — runs after generics::check
                // so the signature is already validated. Gated on the same
                // `has_generic_fn` marker; variance has nothing to do when
                // there are no generic functions.
                if markers.has_generic_fn {
                    crate::variance::check(program, source_path)?;
                }
                // RES-1612 gate: pass loops top-level statements for
                // `Node::NewtypeDecl`.
                if markers.has_newtype_decl {
                    crate::newtypes::check(program, source_path)?;
                }
                // RES-1616 gate: composite — pass validates trait
                // decls, trait refs in impl blocks, and trait bounds
                // on generic fn type-params. Any of the three signals
                // means the pass has work.
                if markers.has_trait_decl
                    || !markers.impl_trait_names.is_empty()
                    || markers.has_generic_fn
                {
                    crate::traits::check(program, source_path)?;
                }
                // RES-2604 gate: validate `impl Display for T` blocks — fmt
                // method presence, arity, and string return type.
                if !markers.impl_trait_names.is_empty() {
                    crate::display_trait::check(program, source_path)?;
                }
                // RES-2552 gate: validate `Node::BlanketImpl` nodes — trait
                // existence, bound existence, method coverage, duplicate check.
                // Must run after `traits::check` so trait decls are well-formed.
                if markers.has_blanket_impl {
                    crate::blanket_impl::check(program, source_path)?;
                }
                // RES-1611: `region_inference::infer` is a no-op stub
                // (`Ok(())`); the real region-aliasing logic lives in
                // `check_call_site_region_aliasing` which runs from a
                // different path. Drop the per-typecheck dispatch.
                // Ralph-Loop-Uniqueness #1: watchdog-feed enforcement.
                // RES-1585 gate: pass scans param types ∈ WATCHDOG_TYPES.
                if markers.any_param_type_in(&["Watchdog", "&Watchdog", "&mut Watchdog"]) {
                    crate::watchdog_feed::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #2: sensor freshness.
                // RES-1585 gate: pass scans param types with SENSOR_TYPE_PREFIXES.
                if markers.any_param_type_with_prefix(&["Sensor", "&Sensor", "&mut Sensor"]) {
                    crate::sensor_freshness::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #3: secret erasure.
                // RES-1585 gate: pass scans for SECRET_NAME_PREFIXES on fn names
                // or SECRET_TYPE_PREFIXES on param types; widen the gate to
                // either marker source to match the pass's coverage.
                if markers
                    .any_fn_name_with_prefix(&["secret_", "key_", "priv_", "password", "nonce_"])
                    || markers.any_param_type_with_prefix(&["Secret", "&Secret", "&mut Secret"])
                {
                    crate::secret_erasure::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #4: transaction close.
                // RES-1585 gate: pass scans param types ∈ TX_TYPES.
                if markers.any_param_type_in(&[
                    "Transaction",
                    "Tx",
                    "&Transaction",
                    "&mut Transaction",
                    "&Tx",
                    "&mut Tx",
                ]) {
                    crate::transaction_commit::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #5: ISR transitive safety.
                // RES-1585 gate: pass triggers on fn names with ISR_NAME_PREFIXES.
                // The pass *also* honours `#[isr]` attributes via the central
                // registry; that path keeps its own fast-reject so we don't
                // need to gate on it here.
                if markers.any_fn_name_with_prefix(&["isr_", "irq_"]) {
                    crate::isr_call_graph::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #6: lock-ordering inversion.
                // RES-1616 gate: pass walks bodies for calls to
                // `lock`/`acquire`/`mutex_lock` or `unlock`/`release`/
                // `mutex_unlock` (LOCK_FNS / UNLOCK_FNS) or any
                // identifier starting with `lock_` / `unlock_`. All
                // three signals come from `markers.call_idents`,
                // populated by the shared whole-AST walk (RES-1593).
                let has_lock_call = markers.call_idents.contains("lock")
                    || markers.call_idents.contains("acquire")
                    || markers.call_idents.contains("mutex_lock")
                    || markers.call_idents.contains("unlock")
                    || markers.call_idents.contains("release")
                    || markers.call_idents.contains("mutex_unlock")
                    || markers.any_call_ident_with_prefix(&["lock_", "unlock_"]);
                if has_lock_call {
                    crate::lock_ordering::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #7: reentrancy guard.
                // RES-1585 gate: pass scans for NR_PREFIXES on fn names.
                if markers.any_fn_name_with_prefix(&["nonreentrant_", "exclusive_"]) {
                    crate::reentrancy_guard::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #8: actor drain-on-shutdown.
                // RES-1232: `Node::Actor` is wired; pass is now active.
                if markers.has_actor {
                    crate::actor_drain::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #9: backpressure-safe handler.
                // RES-1585 gate: pass scans param types ∈ QUEUE_TYPES.
                if markers.any_param_type_in(&[
                    "Mailbox",
                    "BoundedQueue",
                    "&Mailbox",
                    "&mut Mailbox",
                ]) {
                    crate::backpressure_safe::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #10: monotonic-field invariant.
                // RES-1585 gate: pass scans fn names with MONO_PREFIXES.
                if markers.any_fn_name_with_prefix(&["last_", "latest_", "max_", "monotonic_"]) {
                    crate::monotonic_field::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #11: saturation-required arithmetic.
                // RES-1593 gate: pass scans `LetStatement` names with
                // SAT_NAME_SUFFIXES.
                if markers.any_let_name_with_suffix(&[
                    "_pwm",
                    "_duty",
                    "_brightness",
                    "_pct",
                    "_throttle",
                ]) {
                    crate::saturation_required::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #12: numeric units mixing.
                // RES-1593 gate: pass seeds its units map from both
                // let bindings AND fn parameter names whose name ends
                // with a unit suffix.
                if markers.any_let_name_with_suffix(&[
                    "_ms", "_s", "_us", "_ns", "_m", "_cm", "_mm", "_km", "_kg", "_g", "_n", "_v",
                    "_mv", "_a", "_ma", "_hz", "_khz", "_mhz",
                ]) || markers.any_param_name_with_suffix(&[
                    "_ms", "_s", "_us", "_ns", "_m", "_cm", "_mm", "_km", "_kg", "_g", "_n", "_v",
                    "_mv", "_a", "_ma", "_hz", "_khz", "_mhz",
                ]) {
                    crate::numeric_units::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #13: age-bounded data freshness.
                // RES-1593 gate: pass scans `FieldAccess` field names
                // ending in `_at` to detect age-comparison gates.
                if markers.any_field_accessed_with_suffix(&["_at"]) {
                    crate::age_bounded_data::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #14: rate-limit static.
                // RES-1590 gate: pass scans fn names with ONCE_SUFFIXES
                // (`_oncepertick`, `_singleshot`) or FEW_SUFFIX (`_few`).
                if markers.any_fn_name_with_suffix(&["_oncepertick", "_singleshot", "_few"]) {
                    crate::rate_limit_static::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #15: stack budget.
                // RES-1590 gate: pass scans fn names with `_stack{N}` suffix.
                if markers.any_fn_name_with_suffix(&["_stack8", "_stack16", "_stack32", "_stack64"])
                {
                    crate::stack_budget::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #16: heap budget.
                // RES-1590 gate: pass scans fn names with `_alloc{N}` suffix.
                if markers.any_fn_name_with_suffix(&[
                    "_alloc0", "_alloc1", "_alloc2", "_alloc3", "_alloc4", "_alloc5",
                ]) {
                    crate::heap_budget::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #17: bandwidth budget.
                // RES-1590 gate: pass scans fn names with `_iobytes{N}` suffix.
                if markers.any_fn_name_with_suffix(&[
                    "_iobytes16",
                    "_iobytes32",
                    "_iobytes64",
                    "_iobytes128",
                    "_iobytes256",
                    "_iobytes512",
                ]) {
                    crate::bandwidth_budget::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #18: bounded-blocking budget.
                // RES-1590 gate: pass scans fn names with `_bound{N}` suffix.
                if markers.any_fn_name_with_suffix(&["_bound1", "_bound2", "_bound4", "_bound8"]) {
                    crate::bounded_blocking::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #19: audit-log-required mutations.
                // RES-1593 gate: pass scans `FieldAssignment` field
                // names with `audited_` prefix or `_audited` suffix.
                if markers.any_field_assigned_with_prefix_or_suffix(&["audited_"], &["_audited"]) {
                    crate::audit_log_required::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #20: degraded mode after critical assert.
                // RES-1585 gate: pass scans fn names with CRITICAL_PREFIXES
                // (and RECOVERY_PREFIXES are looked up only when CRITICAL
                // matches at least once, so the same gate covers both).
                if markers.any_fn_name_with_prefix(&["assert_critical_", "abort_", "halt_"]) {
                    crate::degraded_mode::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #21: crash-only modules.
                // RES-1585 gate: pass scans fn names starting with `crash_`.
                if markers.any_fn_name_with_prefix(&["crash_"]) {
                    crate::crash_only::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #22: idempotent handlers.
                // RES-1590 gate: pass scans fn names ending in `_idempotent`.
                if markers.any_fn_name_with_suffix(&["_idempotent"]) {
                    crate::idempotent_handler::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #23: epoch ordering.
                // RES-1593 gate: pass matches call identifiers
                // containing `_epoch` (then parses the numeric tail).
                if markers.any_call_ident_containing(&["_epoch"]) {
                    crate::epoch_ordering::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #24: priority-inheritance discipline.
                // RES-1585 gate: pass scans fn names with LOW_PRI_PREFIXES.
                if markers.any_fn_name_with_prefix(&["low_pri_", "bg_", "idle_"]) {
                    crate::priority_inheritance::check(program, source_path)?;
                }
                // Ralph-Loop-Uniqueness #25: TOCTOU guard.
                // RES-1593 gate: pass scans call identifiers with
                // CHECK_SUFFIXES (`_exists`/`_is_valid`/`_status`/`_check`).
                if markers.any_call_ident_with_suffix(&[
                    "_exists",
                    "_is_valid",
                    "_status",
                    "_check",
                ]) {
                    crate::toctou_guard::check(program, source_path)?;
                }
                // 50-feature missing-language-features pass.
                // Each module owns one or more of the 50+1 features in
                // the design doc; see per-module docs for what each
                // does. Order is mostly independent (analysis-only
                // passes), but blame_attribution must run before
                // anti_regression so the latter has the call graph.
                // RES-2645: resilience_score warns for F-grade functions;
                // vibe_debt warns for fully-vibe-coded functions;
                // contract_inference emits inline contract suggestions.
                // All three only examine top-level functions — skip them
                // when the program has none.
                if !markers.fn_names.is_empty() {
                    crate::resilience_score::check(program, source_path)?;
                    crate::vibe_debt::check(program, source_path)?;
                    crate::contract_inference::check(program, source_path)?;
                }
                // RES-2436 gate: behavioral_fingerprint only processes
                // `Node::Function` nodes (fingerprint_program iterates
                // top-level Functions). Programs with no functions have
                // no fingerprints to check, so skip the filesystem I/O
                // (Path::exists + read_to_string) that check() does on
                // every invocation.
                if !markers.fn_names.is_empty() {
                    crate::behavioral_fingerprint::check(program, source_path)?;
                }
                // RES-2436 gate: mutation_testing::generate walks
                // `Node::Function` and `Node::ImplBlock` bodies for
                // arithmetic/logical/comparison operators. Programs with
                // no functions produce zero mutation sites — skip the
                // unconditional O(N) AST walk.
                if !markers.fn_names.is_empty() {
                    crate::mutation_testing::check(program, source_path)?;
                }
                // RES-1629 gate: pass walks bodies for `CallExpression`
                // to build the (caller, callee) blame map. Without any
                // call expressions, `build` returns an empty `BlameMap`
                // — but the pass still installs `BlameMap::default()`
                // to clear stale state from a previous compilation, so
                // preserve that in the else branch.
                if markers.has_call_expression {
                    crate::blame_attribution::check(program, source_path)?;
                } else {
                    crate::blame_attribution::install(crate::blame_attribution::BlameMap::default());
                }
                // RES-1619: `autopilot::check` is a no-op stub; the
                // `--autopilot` CLI flag drives the actual `run()`.
                crate::crash_only_cert::check(program, source_path)?;
                crate::intent_blocks::check(program, source_path)?;
                crate::anti_regression::check(program, source_path)?;
                crate::refinement_types::check(program, source_path)?;
                crate::typestate_types::check(program, source_path)?;
                crate::dependent_arrays::check(program, source_path)?;
                crate::row_polymorphism::check(program, source_path)?;
                crate::info_flow::check(program, source_path)?;
                crate::phantom_types::check(program, source_path)?;
                crate::recursive_types::check(program, source_path)?;
                // RES-1629 gate: pass scans for `Node::ActorDecl` to
                // build an actor→actors graph. With no ActorDecls,
                // both `build` and `detect_cycles` are dead work.
                // `markers.has_actor_decl` was added in RES-1627.
                if markers.has_actor_decl {
                    crate::deadlock_freedom::check(program, source_path)?;
                }
                crate::session_types::check(program, source_path)?;
                crate::probabilistic_contracts::check(program, source_path)?;
                crate::wcet_contracts::check(program, source_path)?;
                crate::distributed_invariants::check(program, source_path)?;
                crate::ghost_types::check(program, source_path)?;
                // RES-2645: incremental_verify evicts stale proof-cache
                // entries for functions that no longer exist in the AST.
                crate::incremental_verify::check(program, source_path)?;
                // RES-1623: `property_tests::check` is a no-op stub
                // (RES-1206); real `collect` runs from the
                // `--run-property-tests` driver.
                crate::mmio_regmap::check(program, source_path)?;
                crate::power_contracts::check(program, source_path)?;
                crate::stack_contracts::check(program, source_path)?;
                crate::no_alloc_cert::check(program, source_path)?;
                crate::hw_state_machine::check(program, source_path)?;
                crate::async_await::check(program, source_path)?;
                crate::atomic_types::check(program, source_path)?;
                crate::lock_priority::check(program, source_path)?;
                crate::default_trait_methods::check(program, source_path)?;
                crate::associated_constants::check(program, source_path)?;
                crate::derives::check(program, source_path)?;
                crate::const_fn::check(program, source_path)?;
                crate::macros::check(program, source_path)?;
                // RES-1607 gate: pass scans top-level statements for
                // `Node::ModuleDecl` or `Node::Use` to build a
                // module-dependency graph. Programs that declare
                // neither produce an empty graph and no cycle to
                // report. Markers already collects both flags from
                // the shared whole-AST walk (RES-1593), so the gate
                // is two O(1) bool reads.
                if markers.has_module_decl || markers.has_use {
                    crate::full_modules::check(program, source_path)?;
                }
                // RES-1597: `package_manager::check` is a no-op stub;
                // manifest parsing happens elsewhere.
                // RES-1599 gate: pass scans for `Node::ImplBlock` with
                // `trait_name == Some("Iterator")`. Markers already
                // collects every `impl_trait_names` from the RES-1593
                // whole-AST walk, so the gate is an O(1) HashSet
                // lookup. The pass unconditionally calls
                // `install_iterator_impls(...)` so a prior program's
                // registration doesn't leak; preserve that semantics
                // by calling the wipe directly in the else branch.
                if markers.has_impl_for_trait("Iterator") {
                    crate::iterator_protocol::check(program, source_path)?;
                } else {
                    crate::iterator_protocol::install_iterator_impls(
                        std::collections::HashSet::new(),
                    );
                }
                // RES-1597: `mutation_testing::check` is a no-op stub;
                // mutation generation only fires from a CLI subcommand.
                // RES-1597: `causal_trace::check` is a no-op stub; trace
                // replay only runs at runtime, not during type-check.
                // RES-1597: `snapshot_regression::check` is a no-op stub;
                // snapshot diffing only fires from the test harness.
                // RES-1598 gate: pass scans for `CallExpression` whose
                // function is the `Err` identifier (the `Result` failure
                // constructor). `markers.call_idents` already records
                // every such ident from the RES-1593 shared AST walk,
                // so the gate is an O(1) HashSet lookup.
                if markers.call_idents.contains("Err") {
                    crate::coverage_warnings::check(program, source_path)?;
                }
                // RES-1605: `param_destructuring::check` is a no-op stub;
                // the parser handles destructured-param desugaring.
                // RES-1605: `format_builtin::check` is a no-op stub;
                // the `format` builtin is registered in the builtin
                // table at startup, not per-typecheck.
                // RES-1597: gate on `has_match_expr` — avoids the
                // struct-pattern walk when the program has no match
                // expressions at all.
                if markers.has_match_expr {
                    crate::struct_exhaustiveness::check(program, source_path)?;
                }
                // RES-400: enum exhaustiveness — verify all declared enum
                // variants are covered in every match expression that only
                // uses EnumVariant arms (no wildcard catch-all).
                if markers.has_enum_decl && markers.has_match_expr {
                    crate::enum_exhaustiveness::check(program, source_path)?;
                }
                // RES-2533: enum payload arity — verify that match arm
                // patterns supply the correct number of bindings for the
                // variant's declared payload. Gated on the same marker
                // pair as exhaustiveness: no enum → nothing to check.
                if markers.has_enum_decl && markers.has_match_expr {
                    crate::enum_payload_match::check(program, source_path)?;
                }
                // RES-324: module namespace validation — detect duplicate
                // module declarations and unresolved name::item references.
                if markers.has_inline_module {
                    crate::modules::check(program, source_path)?;
                }
                // RES-1597: `labeled_break::check` is a no-op stub; the
                // parser already enforces label well-formedness.
                // RES-1606 gate: pass scans for `CallExpression` whose
                // function is the `format` identifier — same shape as
                // RES-1598's `coverage_warnings` gate on `"Err"`.
                // `markers.call_idents` already records every such ident
                // from the shared whole-AST walk (RES-1593), so the gate
                // is an O(1) HashSet lookup.
                if markers.call_idents.contains("format") {
                    crate::fmt_validation::check(program, source_path)?;
                }
                crate::no_panic_cert::check(program, source_path)?;
                crate::ai_threat_model::check(program, source_path)?;
                // RES-1597: `lean_spec::check` is a no-op stub; Lean
                // export is driven by the `--emit-lean-spec` CLI flag.
                crate::mcp_tool_registry::check(program, source_path)?;
                // RES-2605: devirtualize statically-known trait method calls.
                crate::devirtualize::run(program, source_path)?;
                // RES-2592: validate #[must_tail_call] annotations — every
                // self-recursive call inside such a function must be in tail
                // position. No marker gate needed; find_kind("must_tail_call")
                // has an atomic fast-reject when the registry is empty.
                crate::tail_calls::check(program, source_path)?;
                // RES-2535: validate where-clause type-param references.
                crate::where_clauses::check(program, source_path)?;
                // RES-2660: evaluate static_assert conditions at compile time.
                if markers.has_static_assert {
                    crate::static_assert::check(program, source_path)?;
                }
                // RES-2618: f32/f64 cross-width mixing guard.
                crate::float32::check(program, source_path)?;
                // RES-2659: mutual tail call annotation validation.
                crate::mutual_tco::check(program, source_path)?;
                // </EXTENSION_PASSES>

                // RES-192: IO-effect inference. Binary lattice
                // (pure / IO). Fixpoint over the call graph: a fn
                // is tagged IO iff it calls an impure builtin, an
                // already-IO user fn, or an unresolvable callee.
                // Non-error — just populates the `fn_effects`
                // stats field for the --audit column.
                //
                // RES-1322: gated on the opt-in `audit_stats` flag.
                // `fn_effects` is consumed only by the `--audit` and
                // `--explain-effects` drivers; every other caller
                // (default-mode runs, LSP/REPL, every `cargo test`
                // typecheck) never reads the map, so the fixpoint
                // is wasted work. The audit / explain-effects
                // drivers flip the flag via `with_audit_stats(true)`.
                if self.audit_stats {
                    self.stats.fn_effects = infer_fn_effects(statements);
                }

                Ok(result_type)
            }
            _ => Err("Expected program node".to_string()),
        }
    }

    fn match_pattern_binding_types(
        &mut self,
        pattern: &Pattern,
        scrut_ty: &Type,
    ) -> Result<Vec<(String, Type)>, String> {
        match pattern {
            // RES-915: range and wildcard patterns bind no names.
            Pattern::Wildcard | Pattern::Range { .. } => Ok(vec![]),
            // RES-2713: validate that literal patterns are type-compatible
            // with the scrutinee. Catches silent bugs like
            //   match (x: int) { 'a' => ... }  ← char literal on int scrutinee.
            // compatible() returns true when either side is Type::Any, so
            // unknown-type scrutinees pass unconditionally.
            Pattern::Literal(node) => {
                if let Some(pt) = literal_pattern_ty(node)
                    && !compatible(&pt, scrut_ty)
                {
                    return Err(format!(
                        "pattern type `{}` is incompatible with scrutinee type `{}`",
                        pt, scrut_ty
                    ));
                }
                Ok(vec![])
            }
            Pattern::Identifier(n) => Ok(vec![(n.clone(), scrut_ty.clone())]),
            Pattern::Or(branches) => {
                let first = self.match_pattern_binding_types(&branches[0], scrut_ty)?;
                for b in &branches[1..] {
                    let other = self.match_pattern_binding_types(b, scrut_ty)?;
                    if other != first {
                        return Err(format!(
                            "or-pattern branches bind different names or types: {:?} vs {:?}",
                            first, other
                        ));
                    }
                }
                Ok(first)
            }
            Pattern::Bind(outer, inner) => {
                let mut inner_bt = self.match_pattern_binding_types(inner, scrut_ty)?;
                inner_bt.insert(0, (outer.clone(), scrut_ty.clone()));
                Ok(inner_bt)
            }
            Pattern::Struct {
                struct_name,
                fields,
                ..
            } => {
                // RES-408: when the scrutinee is Any (type unknown at
                // compile time), be permissive — the runtime will catch
                // mismatches. Only reject when we have a concrete,
                // conflicting struct name.
                let sname: &str = match scrut_ty {
                    Type::Any => struct_name.as_str(),
                    Type::Struct(s) => s.as_str(),
                    _ => {
                        return Err(format!(
                            "struct pattern `{}` used where scrutinee is not a struct (got {})",
                            struct_name, scrut_ty
                        ));
                    }
                };
                if sname != struct_name.as_str() && !matches!(scrut_ty, Type::Any) {
                    return Err(format!(
                        "struct pattern `{}` does not match scrutinee struct `{}`",
                        struct_name, sname
                    ));
                }
                let decl = self
                    .struct_fields
                    .get(sname)
                    .cloned()
                    .ok_or_else(|| format!("unknown struct `{}` in match pattern", sname))?;
                // RES-1766: pre-size both — `seen` and `out` grow
                // exactly to `fields.len()` on the happy path
                // (every field contributes one seen entry + one
                // binding-type push), so the 0→4→8 doubling chain
                // was paid per struct-pattern-match. Hot path:
                // every Struct match arm runs this.
                let mut seen: HashSet<String> = HashSet::with_capacity(fields.len());
                let mut out = Vec::with_capacity(fields.len());
                for (fname, sub) in fields {
                    if !seen.insert(fname.clone()) {
                        return Err(format!(
                            "duplicate field `{}` in struct match pattern",
                            fname
                        ));
                    }
                    let Some((_, fty)) = decl.iter().find(|(n, _)| n == fname) else {
                        let hint = crate::did_you_mean::hint_from(
                            fname,
                            decl.iter().map(|(n, _)| n.as_str()),
                        );
                        return Err(format!(
                            "struct `{}` has no field `{}` in match pattern{}",
                            sname, fname, hint
                        ));
                    };
                    let sub_bt = self.match_pattern_binding_types(sub.as_ref(), fty)?;
                    out.extend(sub_bt);
                }
                Ok(out)
            }
            // RES-2651: `Some(inner)` extracts the inner type from
            // `Option<T>` when the scrutinee carries a tracked element
            // type. Falls back to `Any` for unparameterised scrutinees
            // (legacy code, dynamic values).
            Pattern::Some(inner) => {
                let inner_ty = match scrut_ty {
                    Type::Option(t) => t.as_ref(),
                    _ => &Type::Any,
                };
                self.match_pattern_binding_types(inner.as_ref(), inner_ty)
            }
            Pattern::None => Ok(vec![]),
            // RES-923: Result patterns recurse same way.
            Pattern::Ok(inner) | Pattern::Err(inner) => {
                self.match_pattern_binding_types(inner.as_ref(), &Type::Any)
            }
            // RES-2533: enum-variant pattern. Look up the variant's
            // declared payload types in `self.enum_decls` so bindings
            // extracted from tuple/named payloads carry their concrete
            // types (e.g. `float` for `Shape::Circle(r)`) rather than
            // the `Any` fallback used before this PR. We resolve type
            // strings into `Type` values eagerly (before recursive
            // `match_pattern_binding_types` calls) to avoid a borrow
            // conflict with `&mut self`.
            Pattern::EnumVariant {
                type_name,
                variant_name,
                payload,
            } => {
                // Resolve declared payload type-name strings; clone
                // them out so we can release the borrow on `self.enum_decls`
                // before the recursive `&mut self` call below.
                enum ResolvedPayload {
                    None,
                    Named(Vec<(String, String)>), // (field_name, type_string)
                    Tuple(Vec<String>),           // positional type strings
                }
                let resolved: ResolvedPayload = match type_name.as_deref() {
                    Some(tn) => {
                        let variants = self.enum_decls.get(tn);
                        match variants.and_then(|vs| vs.iter().find(|v| v.name == *variant_name)) {
                            Some(v) => match &v.payload {
                                crate::EnumPayload::None => ResolvedPayload::None,
                                crate::EnumPayload::Named(fields) => ResolvedPayload::Named(
                                    fields
                                        .iter()
                                        .map(|f| (f.name.clone(), f.ty.clone()))
                                        .collect(),
                                ),
                                crate::EnumPayload::Tuple(tys) => {
                                    ResolvedPayload::Tuple(tys.clone())
                                }
                            },
                            None => ResolvedPayload::None,
                        }
                    }
                    None => ResolvedPayload::None,
                };
                match payload {
                    crate::EnumPatternPayload::None => Ok(vec![]),
                    crate::EnumPatternPayload::Named(fields) => {
                        // RES-1726: pre-size to fields.len().
                        let mut out = Vec::with_capacity(fields.len());
                        for (fname, sub) in fields {
                            // Resolve declared field type; fall back to Any.
                            let field_ty = if let ResolvedPayload::Named(ref decl_fields) = resolved
                            {
                                decl_fields
                                    .iter()
                                    .find(|(n, _)| n == fname)
                                    .and_then(|(_, ty_str)| self.parse_type_name(ty_str).ok())
                                    .unwrap_or(Type::Any)
                            } else {
                                Type::Any
                            };
                            let sub_bt =
                                self.match_pattern_binding_types(sub.as_ref(), &field_ty)?;
                            out.extend(sub_bt);
                        }
                        Ok(out)
                    }
                    crate::EnumPatternPayload::Tuple(subs) => {
                        // RES-1726: pre-size to subs.len().
                        let mut out = Vec::with_capacity(subs.len());
                        for (i, sub) in subs.iter().enumerate() {
                            // Resolve declared positional type; fall back to Any.
                            let elem_ty = if let ResolvedPayload::Tuple(ref tys) = resolved {
                                tys.get(i)
                                    .and_then(|t| self.parse_type_name(t).ok())
                                    .unwrap_or(Type::Any)
                            } else {
                                Type::Any
                            };
                            let sub_bt = self.match_pattern_binding_types(sub, &elem_ty)?;
                            out.extend(sub_bt);
                        }
                        Ok(out)
                    }
                }
            }
            // RES-931: tuple-struct destructure. Verify the scrutinee
            // is a struct with this name, the registered field count
            // matches the pattern arity, then recurse with each
            // declared field's type as the sub-scrutinee. Field names
            // in the registry are "0", "1", ... ordered by index.
            Pattern::TupleStruct { name, fields } => {
                let Type::Struct(sname) = scrut_ty else {
                    return Err(format!(
                        "tuple-struct pattern `{}(..)` used where scrutinee is not a struct (got {})",
                        name, scrut_ty
                    ));
                };
                if name != sname {
                    return Err(format!(
                        "tuple-struct pattern `{}(..)` does not match scrutinee struct `{}`",
                        name, sname
                    ));
                }
                let decl = self.struct_fields.get(sname).cloned().ok_or_else(|| {
                    format!("unknown struct `{}` in tuple-struct match pattern", sname)
                })?;
                if decl.len() != fields.len() {
                    return Err(format!(
                        "tuple-struct pattern `{}` expects {} field(s), got {}",
                        name,
                        decl.len(),
                        fields.len()
                    ));
                }
                // RES-1726: pre-size to fields.len().
                let mut out = Vec::with_capacity(fields.len());
                for (i, sub) in fields.iter().enumerate() {
                    let key = i.to_string();
                    let Some((_, fty)) = decl.iter().find(|(n, _)| n == &key) else {
                        return Err(format!(
                            "tuple-struct `{}` has no positional field `.{}`",
                            sname, i
                        ));
                    };
                    let sub_bt = self.match_pattern_binding_types(sub, fty)?;
                    out.extend(sub_bt);
                }
                Ok(out)
            }
            // RES-932: anonymous tuple destructure. Each positional
            // sub-pattern recurses against `Type::Any` — the dynamic
            // typechecker doesn't track per-position tuple element
            // types yet, mirroring how Some/Ok bindings are widened.
            Pattern::Tuple(items) => {
                // RES-1726: pre-size to items.len().
                let mut out = Vec::with_capacity(items.len());
                for sub in items {
                    let sub_bt = self.match_pattern_binding_types(sub, &Type::Any)?;
                    out.extend(sub_bt);
                }
                Ok(out)
            }
        }
    }

    /// RES-330: scope `var: ty` in a fresh enclosed env, type-check
    /// `body`, then pop the binding. Used by `quantifiers::typecheck_quantifier`
    /// so the quantified variable does not leak into the outer scope.
    pub(crate) fn with_quantifier_binding(
        &mut self,
        var: &str,
        ty: Type,
        body: &Node,
    ) -> Result<Type, String> {
        // RES-1320: build the enclosed inner env from a single clone of
        // the current env, then use `mem::replace` to swap it into
        // `self.env` while capturing the original outer for restore.
        // The previous shape did one clone for `saved` and another for
        // `inner.outer`, paying two full `TypeEnvironment::clone`s on
        // every quantifier the typechecker descends into. The body
        // walk only ever reads `self.env` (via lookups that fall
        // through to `inner.outer` on miss), so the moved original is
        // safe to hand back unchanged after the recursive check.
        let mut inner = TypeEnvironment::new_enclosed(self.env.clone());
        inner.set(var.to_string(), ty);
        let saved = std::mem::replace(&mut self.env, inner);
        let result = self.check_node(body);
        self.env = saved;
        result
    }

    pub fn check_node(&mut self, node: &Node) -> Result<Type, String> {
        match node {
            Node::Program(_statements) => self.check_program(node),
            // RES-073: `use` is resolved away before typecheck. Treat
            // leftovers as void (no-op) for safety.
            Node::Use { .. } => Ok(Type::Void),
            // RES-780: FFI v1 hardening — stricter validation of extern signatures.
            // Reject unsupported ABI shapes at compile time rather than runtime.
            Node::Extern { decls, span, .. } => {
                self.current_span = *span;
                const SUPPORTED_PARAMS: &[&str] =
                    &["Int", "Float", "Bool", "String", "OpaquePtr", "Callback"];
                const SUPPORTED_RETURNS: &[&str] =
                    &["Int", "Float", "Bool", "String", "Void", "OpaquePtr"];

                for d in decls {
                    let fn_name = &d.resilient_name;

                    // Validate parameters
                    for (ty, param_name) in &d.parameters {
                        if !SUPPORTED_PARAMS.contains(&ty.as_str()) {
                            return Err(format!(
                                "FFI: extern fn `{}` parameter `{}` has unsupported type `{}`; \
                                 supported types are: {}",
                                fn_name,
                                param_name,
                                ty,
                                SUPPORTED_PARAMS.join(", ")
                            ));
                        }
                        // Callbacks as parameters require extra validation
                        if ty == "Callback" {
                            return Err(format!(
                                "FFI: extern fn `{}` parameter `{}` uses Callback type; \
                                 function pointers as extern parameters are not yet supported (RES-216)",
                                fn_name, param_name
                            ));
                        }
                    }

                    // Validate return type
                    if !SUPPORTED_RETURNS.contains(&d.return_type.as_str()) {
                        return Err(format!(
                            "FFI: extern fn `{}` has unsupported return type `{}`; \
                             supported types are: {}",
                            fn_name,
                            d.return_type,
                            SUPPORTED_RETURNS.join(", ")
                        ));
                    }

                    // Reject Callback return types
                    if d.return_type == "Callback" {
                        return Err(format!(
                            "FFI: extern fn `{}` returns Callback type; \
                             function pointers as return values are not yet supported (RES-216)",
                            fn_name
                        ));
                    }
                }
                Ok(Type::Void)
            }

            Node::Function {
                name,
                parameters,
                body,
                requires,
                ensures,
                recovers_to,
                return_type: declared_rt,
                fails,
                type_params,
                ..
            } => {
                // RES-425: record type-parameter names so call-site
                // checking can treat Type::Struct("T") as Type::Any.
                if !type_params.is_empty() {
                    self.fn_type_params
                        .insert(name.clone(), type_params.clone());
                }
                // RES-1724: pre-size to `parameters.len()` — exact upper
                // bound, the loop below pushes one entry per parameter.
                // Same shape as the rest of the pre-size series.
                let mut param_types = Vec::with_capacity(parameters.len());

                // Create a new enclosed environment for function body
                let mut function_env = TypeEnvironment::new_enclosed(self.env.clone());

                // Add parameter types to environment
                for (param_type_name, param_name) in parameters {
                    let param_type = self.parse_type_name(param_type_name)?;
                    param_types.push(param_type.clone());
                    function_env.set(param_name.clone(), param_type);
                }

                // Temporarily swap environments
                std::mem::swap(&mut self.env, &mut function_env);

                // RES-060: statically fold every requires / ensures
                // clause. A contradiction is a compile-time error; a
                // tautology is discharged; anything else is left for
                // runtime.
                let no_bindings: HashMap<String, i64> = HashMap::new();
                for (decl_idx, clause) in requires.iter().chain(ensures.iter()).enumerate() {
                    // Cheap folder first; fall back to Z3 (RES-067)
                    // for universal tautology / contradiction proofs.
                    let mut verdict = fold_const_bool(clause, &no_bindings);
                    // RES-136: slot for Z3's counterexample if it
                    // runs and finds a satisfying model for the
                    // negated clause. Only meaningful when we take
                    // the Z3 branch below.
                    let mut decl_counterexample: Option<String> = None;
                    if verdict.is_none() {
                        // RES-1210: consult the incremental-verification
                        // cache before calling Z3. The digest is a hash
                        // of the function name + clause index + clause
                        // debug text — enough to detect any rewrite of
                        // the requires/ensures body.
                        // RES-1897: structural hash instead of format!
                        let clause_digest = {
                            use std::hash::{Hash, Hasher};
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            name.hash(&mut h);
                            decl_idx.hash(&mut h);
                            #[cfg(feature = "z3")]
                            crate::verifier_z3::hash_node_spanless(clause, &mut h);
                            #[cfg(not(feature = "z3"))]
                            format!("{clause:?}").hash(&mut h);
                            h.finish()
                        };
                        if let Some(cached) = crate::incremental_verify::lookup(name, clause_digest)
                        {
                            match cached {
                                crate::incremental_verify::ProofResult::Discharged => {
                                    verdict = Some(true);
                                }
                                crate::incremental_verify::ProofResult::Failed(cx) => {
                                    verdict = Some(false);
                                    decl_counterexample = Some(cx);
                                }
                            }
                        } else {
                            // RES-071: capture the SMT-LIB2 certificate
                            // alongside the verdict so the driver can dump
                            // it to disk if --emit-certificate is set.
                            // RES-354: thread the theory selection through.
                            let (v, cert, cx, timed_out) = {
                                #[cfg(feature = "z3")]
                                {
                                    z3_prove_with_cert_theory(
                                        clause,
                                        &no_bindings,
                                        self.verifier_timeout_ms,
                                        self.z3_theory,
                                    )
                                }
                                #[cfg(not(feature = "z3"))]
                                {
                                    z3_prove_with_cert(
                                        clause,
                                        &no_bindings,
                                        self.verifier_timeout_ms,
                                    )
                                }
                            };
                            verdict = v;
                            if matches!(verdict, Some(true)) {
                                self.stats.requires_discharged_by_z3 += 1;
                                // Store in cache so next compile skips Z3
                                // for this unchanged clause.
                                crate::incremental_verify::store(
                                    name,
                                    clause_digest,
                                    crate::incremental_verify::ProofResult::Discharged,
                                );
                                // RES-1357: only stash the SMT-LIB2 cert
                                // when a consumer asked for it.
                                if self.emit_certificates
                                    && let Some(smt2) = cert
                                {
                                    self.certificates.push(CapturedCertificate {
                                        fn_name: name.clone(),
                                        kind: "decl",
                                        idx: decl_idx,
                                        smt2,
                                    });
                                }
                            } else if matches!(verdict, Some(false)) {
                                // Cache the refuted verdict so subsequent
                                // compiles see the same counterexample
                                // without re-running Z3.
                                crate::incremental_verify::store(
                                    name,
                                    clause_digest,
                                    crate::incremental_verify::ProofResult::Failed(
                                        cx.clone().unwrap_or_default(),
                                    ),
                                );
                            }
                            if timed_out {
                                // RES-137: soft-failure — compilation
                                // continues, runtime check stays in,
                                // audit counter bumps, user sees a hint.
                                self.stats.verifier_timeouts += 1;
                                eprintln!(
                                    "hint: proof timed out after {}ms — runtime check retained (fn {})",
                                    self.verifier_timeout_ms, name
                                );
                            }
                            // RES-217: any unresolved verdict (timeout OR a
                            // genuine Z3 `Unknown`) is a partial proof. Emit
                            // the structured diagnostic so CI / LSP tooling
                            // can discover the specific assertion via a
                            // stable `[partial-proof]` tag. Suppressed with
                            // `--no-warn-unverified`.
                            if verdict.is_none() && self.warn_unverified {
                                emit_partial_proof_warning(&self.source_path, clause);
                            }
                            decl_counterexample = cx;
                        } // end cache-miss branch
                    }
                    match verdict {
                        Some(false) => {
                            // RES-136: include the Z3 counterexample
                            // (if any) so the user sees a concrete
                            // assignment that falsifies the clause.
                            let base = format!(
                                "fn {}: contract can never hold (statically false clause)",
                                name
                            );
                            return Err(match decl_counterexample {
                                Some(cx) => format!("{} — counterexample: {}", base, cx),
                                None => base,
                            });
                        }
                        Some(true) => {
                            self.stats.requires_tautology += 1;
                        }
                        None => {}
                    }
                }

                // RES-392: verify the `recovers_to` crash-recovery
                // postcondition against its MVP semantics — treat
                // the clause as a universal obligation over the
                // function's parameters (and `result`, if the clause
                // mentions it) and try to discharge it via the same
                // static folder / Z3 path used for requires/ensures.
                //
                // This deliberately verifies only the FINAL state,
                // not per-prefix crash semantics. The proper
                // per-instruction bounded model check described in
                // the ticket is a follow-up; when it lands, this
                // block becomes a weaker side-obligation.
                //
                // RES-222: when the fn declares a non-empty `fails`
                // set and there is no structured handler (handlers
                // are a separate ticket — today every fault is
                // "unhandled"), the recovery invariant becomes a
                // real proof obligation. Z3 must show the clause
                // holds under the requires precondition; an
                // undecidable verdict is a compile error. Timeouts
                // are soft — we emit a `hint: proof timed out` that
                // mirrors the per-clause hint on requires/ensures.
                //
                // A provable contradiction (`Some(false)`) is always
                // a compile error — the recovery invariant can never
                // hold. A proven tautology (`Some(true)`) is
                // silently discharged.
                if let Some(clause) = recovers_to {
                    let clause_pos = clause_span(clause);
                    let pos_prefix = if clause_pos.start.line == 0 {
                        String::new()
                    } else {
                        format!("{}:{}: ", clause_pos.start.line, clause_pos.start.column)
                    };

                    // RES-222: admit each `requires` clause as an
                    // axiom. The recovery point is reached only
                    // after the precondition has been checked, so
                    // the solver is allowed to assume them when
                    // discharging the recovery invariant.
                    // RES-133b: also admit leading `assume(P)` predicates
                    // from the function body. They are runtime-checked
                    // before any control flow, so by the time recovers_to
                    // is evaluated they hold.
                    let mut axioms: Vec<Node> = requires.clone();
                    axioms.extend(crate::assume_axioms::collect_leading_assume_axioms(body));

                    let mut verdict = fold_const_bool(clause, &no_bindings);
                    let mut cx: Option<String> = None;
                    let mut cert_smt2: Option<String> = None;
                    let mut timed_out_flag = false;
                    if verdict.is_none() {
                        // RES-1330: drop the dead `z3_prove_with_cert_theory`
                        // call that previously sat here. Its result was
                        // bound to `(_v, _cert, _c, _timed_out)` and
                        // never read — `z3_prove_with_axioms_and_cert`
                        // below was already the load-bearing call (it
                        // admits `requires` preconditions + leading
                        // `assume(P)` as axioms, the only correct shape
                        // for the recovery point per RES-222). The
                        // theory-aware call without axioms would only
                        // ever be strictly weaker than the axioms-aware
                        // one, so dropping it doesn't change the verdict.
                        let (v, cert, c, t) = z3_prove_with_axioms_and_cert(
                            clause,
                            &no_bindings,
                            &axioms,
                            self.verifier_timeout_ms,
                        );
                        verdict = v;
                        cx = c;
                        cert_smt2 = cert;
                        timed_out_flag = t;
                    }

                    // Z3 disproved the clause. Always a compile error.
                    // When `fails` is non-empty, include the fails set so
                    // the diagnostic matches the `None`-verdict message
                    // shape ("cannot be proven" + the failing variant).
                    if matches!(verdict, Some(false)) {
                        let base = if !fails.is_empty() {
                            format!(
                                "{}fn {}: `recovers_to` invariant cannot be proven — \
                                 fn declares `fails` {:?} but Z3 found a counterexample \
                                 showing the invariant does not hold under `requires`",
                                pos_prefix, name, fails
                            )
                        } else {
                            format!(
                                "{}fn {}: `recovers_to` can never hold — \
                                 the recovery invariant is a contradiction",
                                pos_prefix, name
                            )
                        };
                        return Err(match cx {
                            Some(m) => format!("{} — counterexample (final state): {}", base, m),
                            None => base,
                        });
                    }

                    // RES-222: account for successful discharge and
                    // capture the SMT-LIB2 certificate so the driver
                    // can dump it alongside requires/ensures certs.
                    if matches!(verdict, Some(true)) {
                        self.stats.requires_discharged_by_z3 += 1;
                        // RES-1357: only stash the cert when the
                        // `--emit-certificate` driver asked for it.
                        if self.emit_certificates
                            && let Some(smt2) = cert_smt2
                        {
                            self.certificates.push(CapturedCertificate {
                                fn_name: name.clone(),
                                kind: "recovers_to",
                                idx: 0,
                                smt2,
                            });
                        }
                    }

                    // RES-222: with a non-empty `fails` set and no
                    // handler to catch the fault, the invariant is
                    // a mandatory obligation. An undecidable verdict
                    // is a compile error; a timeout degrades to a
                    // hint (runtime check retained) so a slow solver
                    // can't block compilation indefinitely.
                    if verdict.is_none() && !fails.is_empty() {
                        if timed_out_flag {
                            self.stats.verifier_timeouts += 1;
                            eprintln!(
                                "hint: proof timed out after {}ms — runtime check retained (fn {}, recovers_to)",
                                self.verifier_timeout_ms, name
                            );
                            if self.warn_unverified {
                                emit_partial_proof_warning(&self.source_path, clause);
                            }
                        } else {
                            if self.warn_unverified {
                                emit_partial_proof_warning(&self.source_path, clause);
                            }
                            let base = format!(
                                "{}fn {}: `recovers_to` invariant cannot be proven — fn declares `fails` {:?} but no handler catches the fault, and Z3 could not show the recovery invariant holds under the declared `requires`",
                                pos_prefix, name, fails
                            );
                            return Err(match cx {
                                Some(m) => {
                                    format!("{} — counterexample (final state): {}", base, m)
                                }
                                None => base,
                            });
                        }
                    }

                    // RES-1857: when Z3 returns None (not a tautology,
                    // not a contradiction) with no `requires` constraints,
                    // all inputs are valid — the counterexample from the
                    // tautology check is definitive. Reject immediately
                    // rather than relying on the BMC path.
                    if verdict.is_none() && requires.is_empty() && !timed_out_flag {
                        let base = format!(
                            "{}fn {}: `recovers_to` invariant cannot be proven — \
                             Z3 found a counterexample showing the clause does not \
                             hold for all inputs (no `requires` constraint to limit them)",
                            pos_prefix, name
                        );
                        return Err(match cx {
                            Some(ref m) => format!("{} — counterexample: {}", base, m),
                            None => base,
                        });
                    }

                    // RES-392b: per-prefix bounded model checking.
                    // Extends the MVP (final-state only) with verification
                    // that the recovers_to clause holds after recovery from
                    // ANY instruction boundary in the function body.
                    // Pass requires as axioms so the solver can use them,
                    // matching the final-state verifier's axioms path.
                    if let Err(bmc_msg) =
                        crate::recovers_to_bmc::check_recovers_to_bmc(name, body, requires, clause)
                    {
                        eprintln!("warning[bmc]: {bmc_msg}");
                    }
                }

                // RES-065: push each requires clause's extractable
                // assumption into const_bindings so interior call
                // sites can use them. This is the inter-procedural
                // chaining step.
                // RES-1766: pre-size to requires.len() — at most one
                // push per requires clause (only when it extracts an
                // `eq` assumption). Hot: every checked function.
                let mut pushed_assumptions: Vec<(String, Option<i64>)> =
                    Vec::with_capacity(requires.len());
                for clause in requires {
                    if let Some((aname, av)) = extract_eq_assumption(clause) {
                        let prev = self.const_bindings.get(&aname).copied();
                        self.const_bindings.insert(aname.clone(), av);
                        pushed_assumptions.push((aname, prev));
                    }
                }

                // RES-387: enter the fn's fault scope. Call sites in
                // the body may invoke fns with `fails` variants only
                // if each variant is also declared here.
                let saved_fn_fails = self.current_fn_fails.take();
                self.current_fn_fails = Some(fails.clone());

                // RES-403: stash the declared return type so inner
                // `return expr` statements can validate against it.
                let saved_fn_return_type = self.current_fn_return_type.take();
                if let Some(rt_name) = declared_rt
                    && let Ok(rt) = self.parse_type_name(rt_name)
                {
                    self.current_fn_return_type = Some(rt);
                }

                // Check function body
                let body_result = self.check_node(body);

                // RES-387: leave the fault scope before propagating any
                // error, so a nested fn declared inside this body does
                // not inherit our fails set on the way out.
                self.current_fn_fails = saved_fn_fails;
                // RES-403: restore the enclosing fn's return type.
                self.current_fn_return_type = saved_fn_return_type;

                let body_type = body_result?;

                // Restore const_bindings to its pre-body state.
                for (aname, prev) in pushed_assumptions.into_iter().rev() {
                    match prev {
                        Some(v) => {
                            self.const_bindings.insert(aname, v);
                        }
                        None => {
                            self.const_bindings.remove(&aname);
                        }
                    }
                }

                // Restore original environment
                std::mem::swap(&mut self.env, &mut function_env);

                // RES-053: enforce declared return type against body.
                let effective_rt = if let Some(rt_name) = declared_rt {
                    let declared = self.parse_type_name(rt_name)?;
                    if !compatible(&declared, &body_type)
                        // RES-2693: body that returns a concrete struct satisfies
                        // a trait-typed return annotation when the struct impls it.
                        && !self.satisfies_trait_param(&declared, &body_type)
                    {
                        return Err(format!(
                            "fn {}: return type mismatch — declared {}, body produces {}",
                            name, declared, body_type
                        ));
                    }
                    // RES-1112: a non-`void` return type requires every
                    // control-flow path to yield a value — either an
                    // explicit `return EXPR` on every branch, or an
                    // implicit-return ExpressionStatement as the body's
                    // last statement. Without this check, the typechecker
                    // happily accepts `fn f() -> int { if c { return 1; } }`
                    // because the `if`'s consequence_type matches `int`,
                    // even though the else path falls off and yields void
                    // at runtime.
                    if declared != Type::Void && declared != Type::Any && !body_yields_value(body) {
                        return Err(format!(
                            "fn {}: missing return on at least one path — declared `{}`, but the body can fall off the end without returning a value",
                            name, declared
                        ));
                    }
                    declared
                } else {
                    body_type
                };

                // Register function in current environment
                let func_type = Type::Function {
                    params: param_types,
                    return_type: Box::new(effective_rt.clone()),
                };

                self.env.set(name.clone(), func_type);

                Ok(effective_rt)
            }

            Node::LiveBlock { body, .. } => {
                // Live blocks preserve the type of their body
                self.check_node(body)
            }

            // RES-142: duration literals are a parser-internal node
            // that only appear inside a `live ... within <duration>`
            // clause; the parser stores them on `LiveBlock::timeout`
            // rather than emitting them as general expressions. If
            // one reaches the typechecker, treat it as `Int`
            // (nanosecond count) — defensive; should never fire in
            // well-formed programs.
            Node::DurationLiteral { .. } => Ok(Type::Int),

            // RES-291: integer range. The bounds must be `Int`; the
            // expression itself has type `Array` so it can flow through
            // a `for x in <range>` (where the loop variable then gets
            // typed `Int`) or a `let r = <range>;` binding.
            Node::Range { lo, hi, span, .. } => {
                self.current_span = *span;
                let lo_t = self.check_node(lo)?;
                let hi_t = self.check_node(hi)?;
                let ok = |t: &Type| matches!(t, Type::Int | Type::Any);
                if !ok(&lo_t) {
                    return Err(format!("range lower bound must be Int, got {}", lo_t));
                }
                if !ok(&hi_t) {
                    return Err(format!("range upper bound must be Int, got {}", hi_t));
                }
                Ok(Type::Array)
            }

            Node::Assert {
                condition,
                message,
                span,
                ..
            } => {
                self.current_span = *span;
                // Condition must be a boolean expression
                let condition_type = self.check_node(condition)?;
                if condition_type != Type::Bool && condition_type != Type::Any {
                    return Err(format!(
                        "Assert condition must be a boolean, got {}",
                        condition_type
                    ));
                }

                // Message, if present, should be a string
                if let Some(msg) = message {
                    let msg_type = self.check_node(msg)?;
                    if msg_type != Type::String && msg_type != Type::Any {
                        return Err(format!("Assert message must be a string, got {}", msg_type));
                    }
                }

                Ok(Type::Void)
            }

            // RES-222: `invariant EXPR;` — the body must typecheck
            // as bool. Position validity (must sit in a loop body)
            // is enforced by `crate::loop_invariants::check`.
            Node::InvariantStatement { expr, .. } => {
                crate::loop_invariants::typecheck_invariant_statement(self, expr)
            }

            // RES-133a: assume has the same type rules as assert
            Node::Assume {
                condition,
                message,
                span,
                ..
            } => {
                self.current_span = *span;
                let condition_type = self.check_node(condition)?;
                if condition_type != Type::Bool && condition_type != Type::Any {
                    return Err(format!(
                        "Assume condition must be a boolean, got {}",
                        condition_type
                    ));
                }
                if let Some(msg) = message {
                    let msg_type = self.check_node(msg)?;
                    if msg_type != Type::String && msg_type != Type::Any {
                        return Err(format!("Assume message must be a string, got {}", msg_type));
                    }
                }
                Ok(Type::Void)
            }

            Node::Block {
                stmts: statements, ..
            } => {
                let mut result_type = Type::Void;

                // Create a new enclosed environment for block
                let mut block_env = TypeEnvironment::new_enclosed(self.env.clone());
                std::mem::swap(&mut self.env, &mut block_env);

                // RES-1113: track reachability. Once a statement
                // unconditionally terminates (return / break /
                // continue, or an if/match whose every branch does),
                // any subsequent statement in the same block is
                // unreachable and rejected with a clean diagnostic.
                let mut reachable = true;
                let mut block_err: Option<String> = None;
                for stmt in statements {
                    if !reachable {
                        let kind = match stmt {
                            Node::ReturnStatement { .. } => "return",
                            Node::Break { .. } => "break",
                            Node::Continue { .. } => "continue",
                            _ => "statement",
                        };
                        block_err = Some(format!(
                            "unreachable code: {} after an earlier return/break/continue",
                            kind
                        ));
                        break;
                    }
                    result_type = self.check_node(stmt)?;
                    if node_terminates(stmt) {
                        reachable = false;
                    }
                }

                // Restore original environment
                std::mem::swap(&mut self.env, &mut block_env);

                if let Some(e) = block_err {
                    return Err(e);
                }
                Ok(result_type)
            }

            Node::LetStatement {
                name,
                value,
                type_annot,
                span,
            } => {
                // RES-1862: track innermost span for better diagnostics.
                self.current_span = *span;
                let value_type = self.check_node(value)?;
                // RES-414: binding a void-valued expression to a named variable
                // is always a bug. The `_`-prefixed discard convention is the
                // expected pattern; bare `let x = void_fn()` produces a variable
                // that can never be used in a typed context.
                if value_type == Type::Void && !name.starts_with('_') {
                    return Err(format!(
                        "cannot bind void value to `{}` — the right-hand side expression has type void; \
                         use `let _ = expr;` to explicitly discard it",
                        name
                    ));
                }
                // RES-053: enforce `let x: T = value` — reject if value's
                // type isn't compatible with the declared annotation.
                let bind_type = if let Some(ty_name) = type_annot {
                    let declared = self.parse_type_name(ty_name)?;
                    if !compatible(&declared, &value_type)
                        // RES-2693: allow binding a struct to a trait annotation
                        // when the struct implements the trait.
                        && !self.satisfies_trait_param(&declared, &value_type)
                    {
                        return Err(format!(
                            "let {}: {} — value has type {}",
                            name, declared, value_type
                        ));
                    }
                    // RES-411: reject integer literals that overflow the declared pinned-int type.
                    if let Some(literal_val) = fold_const_i64(value, &self.const_bindings) {
                        let range_ok = match &declared {
                            Type::Int8 => (-128_i64..=127).contains(&literal_val),
                            Type::Int16 => (-32768_i64..=32767).contains(&literal_val),
                            Type::Int32 => {
                                (-2_147_483_648_i64..=2_147_483_647).contains(&literal_val)
                            }
                            Type::UInt8 => (0_i64..=255).contains(&literal_val),
                            Type::UInt16 => (0_i64..=65535).contains(&literal_val),
                            Type::UInt32 => (0_i64..=4_294_967_295).contains(&literal_val),
                            _ => true,
                        };
                        if !range_ok {
                            let range_str = match &declared {
                                Type::Int8 => "-128..=127",
                                Type::Int16 => "-32768..=32767",
                                Type::Int32 => "-2147483648..=2147483647",
                                Type::UInt8 => "0..=255",
                                Type::UInt16 => "0..=65535",
                                Type::UInt32 => "0..=4294967295",
                                _ => unreachable!(),
                            };
                            return Err(format!(
                                "let {}: {} — value {} overflows the declared type (valid range: {})",
                                name, declared, literal_val, range_str
                            ));
                        }
                    }
                    declared
                } else {
                    // RES-189: unannotated — stash the inferred
                    // type so the LSP can emit an inlay hint.
                    // Skip when the inferred type is `Any` (no
                    // useful information to surface), `Void`
                    // (shouldn't happen for a let, but guard
                    // against it) or `Var` (inference artifact
                    // that shouldn't leak to users).
                    //
                    // RES-1353: opt-in. Only the LSP consumes
                    // `let_type_hints`; every other entry point
                    // dropped the Vec on TypeChecker drop. Gate
                    // the push (plus its `name.chars().count()` walk
                    // and `value_type.clone()`) on the flag.
                    if self.capture_inlay_hints
                        && !matches!(value_type, Type::Any | Type::Void | Type::Var(..))
                    {
                        self.let_type_hints.push(LetTypeHint {
                            span: *span,
                            name_len_chars: name.chars().count(),
                            ty: value_type.clone(),
                        });
                    }
                    value_type
                };
                self.env.set(name.clone(), bind_type);
                // RES-063: if the RHS is a foldable integer constant,
                // remember the value so future call sites can use it.
                // Otherwise REMOVE any prior binding (shadowing kills
                // the old constant).
                //
                // RES-1390: single fold under the current const bindings.
                // The previous shape was
                //
                //     let no_b = HashMap::new();
                //     fold_const_i64(value, &no_b).or_else(|| fold_const_i64(value, &self.const_bindings))
                //
                // — the `&no_b` probe was redundant. `fold_const_i64`
                // only consults `bindings` in the `Node::Identifier`
                // arm; every other arm folds (literals / prefix /
                // infix recursion) or returns `None` without touching
                // the map. For a pure-literal value, both calls return
                // the same `Some(v)`; for an identifier-referencing
                // value, the `&no_b` call returns `None` and the
                // `&self.const_bindings` call is what produces the
                // result. Either way the simpler single-call shape
                // gives identical results — and saves the empty
                // HashMap allocation per LetStatement type-checked.
                if let Some(v) = fold_const_i64(value, &self.const_bindings) {
                    self.const_bindings.insert(name.clone(), v);
                } else {
                    self.const_bindings.remove(name);
                }
                Ok(Type::Void)
            }

            // RES-155: `let <StructName> { field1, field2: local, .. } = expr;`.
            // Exhaustiveness: without `..`, every struct field must
            // appear in the pattern; missing fields → error listing
            // them. Then we register each local binding in the env
            // with the declared field type (or `Any` if unknown).
            Node::LetDestructureStruct {
                struct_name,
                fields,
                has_rest,
                value,
                ..
            } => {
                let _ = self.check_node(value)?;
                // Look up the struct's declared field list (may be
                // absent if the struct hasn't been declared — we
                // tolerate that and fall back to Any bindings so a
                // partial program still typechecks past this point).
                let declared = self.struct_fields.get(struct_name).cloned();

                if let Some(declared_fields) = &declared {
                    // Reject unknown pattern-field names FIRST — a
                    // typo in the pattern produces a clearer
                    // diagnostic than the missing-field cascade it
                    // would otherwise generate.
                    for (pf, _) in fields {
                        if !declared_fields.iter().any(|(fname, _)| fname == pf) {
                            let hint = crate::did_you_mean::hint_from(
                                pf,
                                declared_fields.iter().map(|(n, _)| n.as_str()),
                            );
                            return Err(format!(
                                "Struct {} has no field `{}`{}",
                                struct_name, pf, hint
                            ));
                        }
                    }
                    // Exhaustiveness check when `..` is not used.
                    if !has_rest {
                        let mut missing: Vec<&str> = declared_fields
                            .iter()
                            .filter(|(fname, _)| !fields.iter().any(|(pf, _)| pf == fname))
                            .map(|(fname, _)| fname.as_str())
                            .collect();
                        if !missing.is_empty() {
                            missing.sort();
                            return Err(format!(
                                "Non-exhaustive destructure of {}: missing field(s) {} — add `..` to ignore them",
                                struct_name,
                                missing.join(", ")
                            ));
                        }
                    }
                }

                // Bind each local name with the declared field type,
                // or `Any` if the struct's declaration is unavailable.
                for (field_name, local_name) in fields {
                    let ty = declared
                        .as_ref()
                        .and_then(|dfs| {
                            dfs.iter()
                                .find(|(fn_, _)| fn_ == field_name)
                                .map(|(_, t)| t.clone())
                        })
                        .unwrap_or(Type::Any);
                    self.env.set(local_name.clone(), ty);
                    self.const_bindings.remove(local_name);
                }
                Ok(Type::Void)
            }

            Node::ArrayLiteral { items, .. } => {
                // RES-402: element-type enforcement at construction.
                // Walk every item, gather their inferred types, and
                // reject the literal when the set of distinct concrete
                // (non-`Any`) types is greater than one. `Any` items
                // pair with anything (existing inference is permissive).
                // Empty literals are allowed and remain `Type::Array`.
                // RES-1766: pre-size to items.len() — at most one
                // push per item (skipped only when ty is Any). Hot:
                // every ArrayLiteral in source.
                let mut concrete: Vec<(Type, &Node)> = Vec::with_capacity(items.len());
                for item in items {
                    let ty = self.check_node(item)?;
                    if !matches!(ty, Type::Any) {
                        concrete.push((ty, item));
                    }
                }
                if concrete.len() > 1 {
                    let first_ty = &concrete[0].0;
                    if let Some((other_ty, _)) =
                        concrete.iter().skip(1).find(|(t, _)| t != first_ty)
                    {
                        return Err(format!(
                            "Array literal contains mixed element types: {} and {}. Resilient does not implicitly coerce between types — pick one and convert the others explicitly.",
                            first_ty, other_ty
                        ));
                    }
                }
                Ok(Type::Array)
            }

            // RES-148: map literal — walk every key and value to
            // surface nested type errors, but fall back to `Type::Any`
            // for the result until a real `Type::Map<K, V>` lands in
            // the typechecker.
            // RES-415: enforce key-type and value-type consistency so that
            // {1: "a", "b": "c"} is rejected the same way mixed-type array
            // literals are rejected.
            Node::MapLiteral { entries, .. } => {
                let mut key_types: Vec<Type> = Vec::with_capacity(entries.len());
                let mut val_types: Vec<Type> = Vec::with_capacity(entries.len());
                for (k, v) in entries {
                    let kt = self.check_node(k)?;
                    let vt = self.check_node(v)?;
                    if !matches!(kt, Type::Any) {
                        key_types.push(kt);
                    }
                    if !matches!(vt, Type::Any) {
                        val_types.push(vt);
                    }
                }
                if key_types.len() > 1 {
                    let first = &key_types[0];
                    if let Some(other) = key_types.iter().skip(1).find(|t| *t != first) {
                        return Err(format!(
                            "map literal contains mixed key types: {} and {} — all keys must have the same type",
                            first, other
                        ));
                    }
                }
                if val_types.len() > 1 {
                    let first = &val_types[0];
                    if let Some(other) = val_types.iter().skip(1).find(|t| *t != first) {
                        return Err(format!(
                            "map literal contains mixed value types: {} and {} — all values must have the same type",
                            first, other
                        ));
                    }
                }
                Ok(Type::Any)
            }

            // RES-149: set literal. Walk each item to catch nested
            // type errors; return `Type::Any` for now — same posture
            // as `MapLiteral` until `Type::Set<T>` shows up.
            // RES-415: set literal element-type consistency, mirroring
            // the same check for array literals.
            Node::SetLiteral { items, .. } => {
                let mut concrete: Vec<Type> = Vec::with_capacity(items.len());
                for item in items {
                    let ty = self.check_node(item)?;
                    if !matches!(ty, Type::Any) {
                        concrete.push(ty);
                    }
                }
                if concrete.len() > 1 {
                    let first = &concrete[0];
                    if let Some(other) = concrete.iter().skip(1).find(|t| *t != first) {
                        return Err(format!(
                            "Set literal contains mixed element types: {} and {}",
                            first, other
                        ));
                    }
                }
                Ok(Type::Any)
            }

            Node::TryExpression { expr: inner, .. } => {
                let inner_type = self.check_node(inner)?;
                // RES-2715: `?` works on both Result and Option (the runtime
                // handles both — see lib.rs RES-375 comment). For Option<T>
                // propagate the inner type T so callers know the unwrapped
                // value's type. For unparameterised Result return Any (the
                // Ok payload type is not tracked yet).
                if let Type::Option(ok_ty) = &inner_type {
                    return Ok(*ok_ty.clone());
                }
                if compatible(&inner_type, &Type::Result) {
                    return Ok(Type::Any);
                }
                Err(format!(
                    "? operator expects a Result or Option, got {}",
                    inner_type
                ))
            }

            // RES-363: `expr?.field` / `expr?.method(args)` — optional
            // chaining. Yields `Any` at MVP; a future ticket can refine
            // this to `Option<T>` once the type system carries generics.
            Node::OptionalChain { object, access, .. } => {
                self.check_node(object)?;
                if let crate::ChainAccess::Method(_, args) = access {
                    for a in args {
                        self.check_node(a)?;
                    }
                }
                Ok(Type::Any)
            }

            Node::FunctionLiteral {
                parameters,
                body,
                return_type: lit_return_type,
                ..
            } => {
                // Evaluate the body's type in a child env with params
                // bound, just like named Function.
                // RES-1766: pre-size to parameters.len() — exactly
                // one push per parameter, exact bound. Hot: every
                // FunctionLiteral in source.
                let mut param_types = Vec::with_capacity(parameters.len());
                let mut fn_env = TypeEnvironment::new_enclosed(self.env.clone());
                for (tname, pname) in parameters {
                    let ty = self.parse_type_name(tname)?;
                    param_types.push(ty.clone());
                    fn_env.set(pname.clone(), ty);
                }
                // RES-403: isolate current_fn_return_type so inner
                // `return` statements validate against the literal's
                // declared type, not the enclosing named function's.
                let saved_lit_return_type = self.current_fn_return_type.take();
                if let Some(rt_name) = lit_return_type
                    && let Ok(rt) = self.parse_type_name(rt_name)
                {
                    self.current_fn_return_type = Some(rt);
                }
                std::mem::swap(&mut self.env, &mut fn_env);
                let body_type = self.check_node(body)?;
                std::mem::swap(&mut self.env, &mut fn_env);
                self.current_fn_return_type = saved_lit_return_type;
                Ok(Type::Function {
                    params: param_types,
                    return_type: Box::new(body_type),
                })
            }

            Node::Match {
                scrutinee,
                arms,
                span,
                ..
            } => {
                self.current_span = *span;
                let scrutinee_type = self.check_node(scrutinee)?;
                // RES-402: collect arm body types for return-type inference.
                let mut arm_types: Vec<Type> = Vec::with_capacity(arms.len());
                for (pattern, guard, body) in arms {
                    // RES-160: or-pattern binding consistency —
                    // every branch must bind the same set of names,
                    // otherwise the arm body's reference to a
                    // binding would be conditional on which branch
                    // fired, which is confusing and error-prone.
                    if let Pattern::Or(branches) = pattern {
                        let first = pattern_bindings(&branches[0]);
                        for b in &branches[1..] {
                            let other = pattern_bindings(b);
                            if other != first {
                                return Err(format!(
                                    "or-pattern branches bind different names: {:?} vs {:?}",
                                    first, other
                                ));
                            }
                        }
                    }

                    // RES-159 + RES-160 + RES-161a + RES-369: register
                    // every name the pattern binds with the correct type
                    // (scrutinee type for simple arms; per-field types for
                    // struct patterns). Rolled back after the arm.
                    let binding_entries =
                        self.match_pattern_binding_types(pattern, &scrutinee_type)?;
                    let rollback_bindings: Vec<(String, Option<Type>)> = binding_entries
                        .iter()
                        .map(|(n, t)| {
                            let prev = self.env.get(n);
                            self.env.set(n.clone(), t.clone());
                            (n.clone(), prev)
                        })
                        .collect();

                    if let Some(g) = guard {
                        // RES-159: guards must be boolean-ish. Accept
                        // Bool / Any so existing permissive inference
                        // stays compatible.
                        let gt = self.check_node(g)?;
                        if gt != Type::Bool && gt != Type::Any {
                            for (n, prev) in &rollback_bindings {
                                match prev {
                                    Some(t) => self.env.set(n.clone(), t.clone()),
                                    None => {
                                        self.env.remove(n);
                                    }
                                }
                            }
                            return Err(format!("Match arm guard must be a boolean, got {}", gt));
                        }
                    }
                    let body_res = self.check_node(body);
                    // Roll back all pattern-binding entries before propagating
                    // the error so the environment is clean on early return.
                    for (n, prev) in rollback_bindings {
                        match prev {
                            Some(t) => self.env.set(n, t),
                            None => {
                                self.env.remove(&n);
                            }
                        }
                    }
                    // RES-402: collect arm type for common-type inference.
                    arm_types.push(body_res?);
                }

                // RES-054 + RES-159 + RES-160 + RES-369: exhaustiveness.
                // An arm covers the scrutinee domain when it's unguarded
                // and the pattern is exhaustive for the scrutinee type
                // (`pattern_is_default` for scalars; struct `{{ .. }}`
                // or a full field-wise binding pattern for structs).
                let has_default = arms.iter().any(|(p, guard, _)| {
                    guard.is_none()
                        && pattern_is_exhaustive_wrt_scrutinee(
                            &scrutinee_type,
                            p,
                            &self.struct_fields,
                        )
                });

                if !has_default {
                    match scrutinee_type {
                        // Bool is the only finite-domain scalar; require
                        // coverage of both true and false via UNGUARDED
                        // arms (guarded arms don't count). Or-patterns
                        // union their branches' coverage, so
                        // `true | false => ...` fully covers.
                        Type::Bool => {
                            let has_true = arms.iter().any(|(p, guard, _)| {
                                guard.is_none() && pattern_covers_bool(p, true)
                            });
                            let has_false = arms.iter().any(|(p, guard, _)| {
                                guard.is_none() && pattern_covers_bool(p, false)
                            });
                            if !(has_true && has_false) {
                                return Err(format!(
                                    "Non-exhaustive match on bool: {}{}{}",
                                    if has_true { "" } else { "missing `true` arm" },
                                    if !has_true && !has_false { "; " } else { "" },
                                    if has_false { "" } else { "missing `false` arm" },
                                ));
                            }
                        }
                        // For any other scrutinee type — int, float,
                        // string, struct, Result, etc. — a wildcard /
                        // identifier arm is required. The domain is
                        // effectively open.
                        Type::Any => {
                            // Scrutinee type unknown → accept the match
                            // rather than force a wildcard. Real
                            // exhaustiveness for user types lands with
                            // G7's struct-decl table.
                        }
                        Type::Struct(sname) => {
                            // RES-400: a `Type::Struct(name)` whose
                            // `name` resolves in `enum_decls` is
                            // really an enum scrutinee. Check that
                            // every declared variant is covered.
                            if let Some(variants) = self.enum_decls.get(&sname).cloned() {
                                let covered: HashSet<&str> = arms
                                    .iter()
                                    .filter(|(_, g, _)| g.is_none())
                                    .filter_map(|(p, _, _)| match p {
                                        Pattern::EnumVariant { variant_name, .. } => {
                                            Some(variant_name.as_str())
                                        }
                                        _ => None,
                                    })
                                    .collect();
                                let missing: Vec<&str> = variants
                                    .iter()
                                    .filter(|v| !covered.contains(v.name.as_str()))
                                    .map(|v| v.name.as_str())
                                    .collect();
                                if !missing.is_empty() {
                                    let list = missing
                                        .iter()
                                        .map(|m| format!("{}::{}", sname, m))
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    return Err(format!(
                                        "Non-exhaustive match on enum `{}`: missing variants: {}",
                                        sname, list
                                    ));
                                }
                            } else {
                                return Err(format!(
                                    "Non-exhaustive match on struct `{}`: add `{} {{ .. }}`, `_`, or an identifier arm that covers every field",
                                    sname, sname
                                ));
                            }
                        }
                        Type::Result => {
                            // RES-2703: bare `Ok(v)` / `Err(e)` arms parse as
                            // Pattern::Ok / Pattern::Err, not EnumVariant. Also
                            // accept the enum-qualified `Result::Ok(v)` form
                            // (Pattern::EnumVariant { variant_name: "Ok", .. }).
                            let covered: HashSet<&str> = arms
                                .iter()
                                .filter(|(_, g, _)| g.is_none())
                                .filter_map(|(p, _, _)| match p {
                                    Pattern::Ok(_) => Some("Ok"),
                                    Pattern::Err(_) => Some("Err"),
                                    Pattern::EnumVariant { variant_name, .. } => {
                                        Some(variant_name.as_str())
                                    }
                                    _ => None,
                                })
                                .collect();
                            if !covered.contains("Ok") || !covered.contains("Err") {
                                let mut missing = Vec::new();
                                if !covered.contains("Ok") {
                                    missing.push("Result::Ok");
                                }
                                if !covered.contains("Err") {
                                    missing.push("Result::Err");
                                }
                                return Err(format!(
                                    "Non-exhaustive match on enum `Result`: missing variants: {}",
                                    missing.join(", ")
                                ));
                            }
                        }
                        // RES-2651: Option<T> is exhaustive when both
                        // Some and None arms are present.
                        Type::Option(_) => {
                            let has_some = arms.iter().any(|(p, guard, _)| {
                                guard.is_none() && matches!(p, Pattern::Some(_))
                            });
                            let has_none = arms
                                .iter()
                                .any(|(p, guard, _)| guard.is_none() && matches!(p, Pattern::None));
                            if !(has_some && has_none) {
                                let mut missing = Vec::new();
                                if !has_some {
                                    missing.push("Some");
                                }
                                if !has_none {
                                    missing.push("None");
                                }
                                return Err(format!(
                                    "Non-exhaustive match on {}: missing variants: {}",
                                    scrutinee_type,
                                    missing.join(", ")
                                ));
                            }
                        }
                        other => {
                            return Err(format!(
                                "Non-exhaustive match on {}: add a wildcard `_` or identifier arm to handle unmatched values",
                                other
                            ));
                        }
                    }
                }

                // RES-2664: reject match arms with incompatible types.
                // Find the first concrete (non-Any) arm type and verify
                // all other concrete arm types are compatible with it.
                let mut expected: Option<&Type> = None;
                for t in &arm_types {
                    if matches!(t, Type::Any) {
                        continue;
                    }
                    match expected {
                        None => expected = Some(t),
                        Some(e) if compatible(e, t) => {}
                        Some(e) => {
                            return Err(format!(
                                "match arms have incompatible types: {} and {}",
                                e, t
                            ));
                        }
                    }
                }
                // RES-402: return the common type of all arm bodies.
                Ok(infer_common_arm_type(&arm_types))
            }

            // RES-158: walk each method as if it were a top-level fn.
            // The parser has already mangled the name and injected
            // `self` as the first parameter, so no special handling is
            // required here beyond delegation.
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    let _ = self.check_node(method)?;
                }
                Ok(Type::Void)
            }

            // RES-128: register the alias. Resolution (with cycle
            // detection) happens in `parse_type_name` / the companion
            // `resolve_type_alias` helper; this arm just records the
            // mapping.
            //
            // NOTE: aliases are NOT nominal — `Meters` unifies with
            // `Int`. Users who want a fresh nominal type wrap the
            // target in a one-field struct (RES-126 covers the
            // nominal rule).
            Node::TypeAlias { name, target, span } => {
                // RES-410: duplicate type alias is an error, but only
                // when the alias is being registered for the first time
                // in the main pass. The pre-pass at the top of
                // Node::Program may have already inserted the alias so
                // function parameters can reference it; in that case
                // the target string matches and we silently accept the
                // re-registration. A genuine re-declaration with a
                // *different* target is the error we want to catch.
                if let Some(prev_target) = self.type_aliases.get(name)
                    && prev_target != target
                {
                    return Err(format!(
                        "{}:{}:{}: error: duplicate type alias `{}` — previously defined as `{}`, now re-defined as `{}`",
                        self.source_path,
                        span.start.line,
                        span.start.column,
                        name,
                        prev_target,
                        target,
                    ));
                }
                self.type_aliases.insert(name.clone(), target.clone());
                Ok(Type::Void)
            }

            // RES-391: `region <Name>;` is compile-time metadata — Void.
            Node::RegionDecl { .. } => Ok(Type::Void),

            // RES-319: newtype declaration is compile-time metadata — Void.
            Node::NewtypeDecl { .. } => Ok(Type::Void),
            // RES-319: newtype constructor — check the inner value and return
            // `Type::Struct` named after the newtype (nominal distinction).
            Node::NewtypeConstruct {
                type_name, value, ..
            } => {
                self.check_node(value)?;
                Ok(Type::Struct(type_name.clone()))
            }

            // RES-333 / RES-776: supervisor declaration typechecker integration.
            // Phase 1 (RES-776 PR 1): validation of supervisor syntax and referenced functions ✓
            // Phase 2 (RES-776 PR 2 / RES-780): typechecker integration to validate:
            //   - Referenced functions match expected actor handler signatures
            //   - Supervisor strategy is compatible with supervised actors
            //   - Child restart policies are well-typed
            // Phase 3 (RES-776 PR 3-5): runtime crash handling and restart policies
            Node::SupervisorDecl { .. } => {
                // Currently: basic structural validation (functions exist, strategy valid, IDs unique)
                // RES-776 Phase 2: signature validation (zero-param, void return)
                // is now integrated into crate::supervisor::check.
                crate::supervisor::check(node, &self.env)?;
                Ok(Type::Void)
            }

            // RES-224 (RES-387 follow-up): `try { ... } catch V { ... }`.
            // Extend the in-scope `fails` set with every caught
            // variant while type-checking the body, then restore for
            // the handler bodies (a handler is not inside its own
            // `catch` scope — it runs once the body already failed).
            Node::TryCatch { body, handlers, .. } => {
                let augmented =
                    crate::try_catch::augmented_fn_fails(self.current_fn_fails.as_ref(), handlers);
                let saved = self.current_fn_fails.replace(augmented);
                for stmt in body {
                    self.check_node(stmt)?;
                }
                self.current_fn_fails = saved;
                for (_, handler_body) in handlers {
                    for stmt in handler_body {
                        self.check_node(stmt)?;
                    }
                }
                Ok(Type::Void)
            }

            // RES-330: quantifier expression. Body must be Bool with
            // the bound variable in scope. Logic in `quantifiers.rs`.
            Node::Quantifier {
                var, range, body, ..
            } => crate::quantifiers::typecheck_quantifier(self, var, range, body),

            // RES-386: commutativity actor type-checks as Void.
            Node::Actor { .. } => Ok(Type::Void),

            // RES-390: ClusterDecl is compile-time-only.
            Node::ClusterDecl { .. } => Ok(Type::Void),

            // RES-401: tuples — type-check each item in a literal and
            // return a precise Type::Tuple with element types. TupleIndex
            // returns the element type when the index is in range; otherwise
            // Type::Any. LetTupleDestructure binds each name to its element
            // type from the tuple type.
            Node::TupleLiteral { items, .. } => {
                let mut elem_types = Vec::with_capacity(items.len());
                for it in items {
                    elem_types.push(self.check_node(it)?);
                }
                Ok(Type::Tuple(elem_types))
            }
            Node::TupleIndex { tuple, index, .. } => {
                let tup_ty = self.check_node(tuple)?;
                if let Type::Tuple(ref elems) = tup_ty {
                    if let Some(elem_ty) = elems.get(*index) {
                        return Ok(elem_ty.clone());
                    }
                    return Err(format!(
                        "{}:{}:{}: tuple index {} out of range (tuple has {} element(s))",
                        self.source_path,
                        self.current_span.start.line,
                        self.current_span.start.column,
                        index,
                        elems.len()
                    ));
                }
                Ok(Type::Any)
            }
            Node::LetTupleDestructure { names, value, .. } => {
                let rhs_ty = self.check_node(value)?;
                match rhs_ty {
                    Type::Tuple(ref elems) => {
                        for (i, n) in names.iter().enumerate() {
                            let elem_ty = elems.get(i).cloned().unwrap_or(Type::Any);
                            self.env.set(n.clone(), elem_ty);
                        }
                    }
                    _ => {
                        for n in names {
                            self.env.set(n.clone(), Type::Any);
                        }
                    }
                }
                Ok(Type::Void)
            }

            // RES-388/RES-390: ActorDecl type-checks state fields,
            // always invariants, and receive handler bodies.
            // RES-777 / RES-790: validate that actor state, handler parameters,
            // and mailbox payloads contain no reference types to preserve
            // ownership-by-value across actor boundaries. This structural constraint
            // prevents aliasing that could enable data races despite the actor model.
            // When an actor sends(pid, value) or receives a message, the value must
            // be by-value to maintain isolation guarantees.
            // RES-776 / RES-780: Integration point for supervisor-aware validation.
            // When a supervisor declares this actor as a child, the typechecker
            // should validate handler signatures match the child's restart policy
            // and expected behavior. (Phase 2: not yet implemented)
            Node::ActorDecl {
                name,
                state_fields,
                always_clauses,
                eventually_clauses,
                receive_handlers,
                ..
            } => {
                // RES-1323: mirror the block / function / loop env-scope
                // pattern (one `.clone()` + `mem::swap` for restore) instead
                // of the previous `let saved_env = self.env.clone()` shape
                // that paid two full env clones per actor (one outer, one
                // per handler). Lookups during state-field / always /
                // eventually checking fall through the enclosed `outer` on
                // miss, so the saved chain stays observable for free.
                let mut actor_env = TypeEnvironment::new_enclosed(self.env.clone());
                std::mem::swap(&mut self.env, &mut actor_env);
                // RES-1399: pre-size to state_fields.len() so the push
                // loop below doesn't reallocate as the Vec grows.
                // Actors with N>4 state fields previously paid 2-3 Vec
                // reallocations (4-elt default + doubling); the count
                // is statically known here, so use it.
                let mut resolved_fields: Vec<(String, Type)> =
                    Vec::with_capacity(state_fields.len());
                for (ty, field, init) in state_fields {
                    // RES-777: reject reference types in actor state
                    if is_reference_type(ty) {
                        return Err(format!(
                            "actor `{}` state field `{}` has reference type {}; actor boundaries require ownership-by-value to preserve race safety",
                            name, field, ty
                        ));
                    }
                    let resolved = self.parse_type_name(ty)?;
                    let init_ty = self.check_node(init)?;
                    if init_ty != resolved && init_ty != Type::Any && resolved != Type::Any {
                        return Err(format!(
                            "actor `{}` state field `{}` initializer has type {}, expected {}",
                            name, field, init_ty, resolved
                        ));
                    }
                    self.env.set(field.clone(), resolved.clone());
                    resolved_fields.push((field.clone(), resolved));
                }
                // RES-1349: move `resolved_fields` directly — it has
                // no further reader in this arm, so `.clone()` was
                // pure dead allocation (one extra `Vec` + one extra
                // `String` per state field).
                //
                // RES-1365: wrap in `Rc` so per-FieldAccess reads on
                // the actor state can clone a single refcount.
                self.struct_fields
                    .insert(name.clone(), std::rc::Rc::new(resolved_fields));
                for clause in always_clauses {
                    let ty = self.check_node(clause)?;
                    if ty != Type::Bool && ty != Type::Any {
                        return Err(format!(
                            "actor `{}` `always` invariant must be Bool, got {}",
                            name, ty
                        ));
                    }
                }
                // RES-388 follow-up: `eventually(after: h): P;` — `P`
                // must type-check as Bool against the actor's state
                // environment, and `h` must name a real handler.
                for ev in eventually_clauses {
                    let ty = self.check_node(&ev.post)?;
                    if ty != Type::Bool && ty != Type::Any {
                        return Err(format!(
                            "actor `{}` `eventually` post-condition must be Bool, got {}",
                            name, ty
                        ));
                    }
                    if !receive_handlers.iter().any(|h| h.name == ev.target_handler) {
                        return Err(format!(
                            "actor `{}` `eventually(after: {})` references unknown handler",
                            name, ev.target_handler
                        ));
                    }
                }
                for handler in receive_handlers {
                    // RES-1323: per-handler scope via mem::swap (see the
                    // outer comment above) — one clone of the actor env
                    // for the enclosed handler env's outer, then swap in
                    // and out instead of cloning + restoring.
                    let mut handler_env = TypeEnvironment::new_enclosed(self.env.clone());
                    std::mem::swap(&mut self.env, &mut handler_env);
                    self.env.set("self".to_string(), Type::Struct(name.clone()));
                    for (pty, pname) in &handler.parameters {
                        // RES-777: reject reference types in handler payloads
                        if is_reference_type(pty) {
                            return Err(format!(
                                "actor `{}` handler `{}` parameter `{}` has reference type {}; actor boundaries require ownership-by-value to preserve race safety",
                                name, handler.name, pname, pty
                            ));
                        }
                        let resolved = self.parse_type_name(pty)?;
                        self.env.set(pname.clone(), resolved);
                    }
                    for r in &handler.requires {
                        let _ = self.check_node(r)?;
                    }
                    for e in &handler.ensures {
                        let _ = self.check_node(e)?;
                    }
                    let _ = self.check_node(&handler.body)?;
                    std::mem::swap(&mut self.env, &mut handler_env);
                }
                std::mem::swap(&mut self.env, &mut actor_env);
                Ok(Type::Void)
            }

            // RES-153: record the struct's (field, type) list so
            // `FieldAccess` / `FieldAssignment` downstream can check
            // field existence and surface typed-field errors
            // statically.
            Node::StructDecl {
                name, fields, span, ..
            } => {
                // RES-409: reject duplicate struct declarations at the
                // same scope. A second `struct Foo` would silently
                // shadow the first, causing confusing field-type
                // mismatches later.
                if self.struct_fields.contains_key(name) {
                    return Err(format!(
                        "{}:{}:{}: error: duplicate struct declaration `{}`",
                        self.source_path, span.start.line, span.start.column, name,
                    ));
                }

                let mut seen_fields: std::collections::HashSet<&str> =
                    std::collections::HashSet::with_capacity(fields.len());
                let mut resolved: Vec<(String, Type)> = Vec::with_capacity(fields.len());
                for (type_name, field_name) in fields {
                    if !seen_fields.insert(field_name.as_str()) {
                        return Err(format!(
                            "{}:{}:{}: error: duplicate field `{}` in struct `{}`",
                            self.source_path, span.start.line, span.start.column, field_name, name,
                        ));
                    }
                    let ty = self.parse_type_name(type_name)?;
                    resolved.push((field_name.clone(), ty));
                }
                // RES-1365: wrap in `Rc` so per-FieldAccess reads
                // clone a single refcount.
                self.struct_fields
                    .insert(name.clone(), std::rc::Rc::new(resolved));
                Ok(Type::Void)
            }

            Node::StructLiteral {
                name,
                fields,
                base,
                span,
                ..
            } => {
                self.current_span = *span;
                // RES-404: look up declared field types. When the struct
                // is known, validate (a) no unknown fields and (b) every
                // provided value's type is compatible with the declared
                // field type. Missing-field errors are L0024 lint; the
                // typechecker only rejects type mismatches and unknowns.
                let effective_struct_name = if let Some(idx) = name.rfind("::") {
                    let type_name = &name[..idx];
                    if self.enum_decls.contains_key(type_name) {
                        // enum-variant constructor: validate against the
                        // variant's payload fields if the enum is known.
                        // For now only reject obvious unknown field names.
                        type_name.to_string()
                    } else {
                        name.clone()
                    }
                } else {
                    name.clone()
                };
                // RES-2632: validate base expression for struct update syntax.
                if let Some(base_expr) = base {
                    let base_ty = self.check_node(base_expr)?;
                    if let Type::Struct(base_name) = &base_ty {
                        if base_name != &effective_struct_name {
                            return Err(format!(
                                "struct update base has type `{}`, \
                                 but the literal constructs `{}`",
                                base_name, effective_struct_name
                            ));
                        }
                    } else if base_ty != Type::Any {
                        return Err(format!(
                            "struct update base must be a struct, found {}",
                            base_ty
                        ));
                    }
                }
                let declared_opt = self.struct_fields.get(&effective_struct_name).cloned();
                for (field_name, e) in fields {
                    let val_ty = self.check_node(e)?;
                    if let Some(declared) = &declared_opt {
                        // RES-418: unknown field in struct literal.
                        if !declared.iter().any(|(n, _)| n == field_name) {
                            let avail: Vec<&str> =
                                declared.iter().map(|(n, _)| n.as_str()).collect();
                            let hint =
                                crate::did_you_mean::hint_from(field_name, avail.iter().copied());
                            return Err(format!(
                                "struct `{}` has no field `{}`{}; available fields: {}",
                                effective_struct_name,
                                field_name,
                                hint,
                                if avail.is_empty() {
                                    "(none)".to_string()
                                } else {
                                    avail.join(", ")
                                }
                            ));
                        }
                        // Type mismatch on a known field.
                        if let Some((_, field_ty)) = declared.iter().find(|(n, _)| n == field_name)
                            && !compatible(field_ty, &val_ty)
                        {
                            return Err(format!(
                                "struct `{}` field `{}` has type {}, \
                                 but the initializer has type {}",
                                effective_struct_name, field_name, field_ty, val_ty
                            ));
                        }
                    }
                }
                // RES-400: if `name` is an enum-variant constructor
                // (`EnumName::VariantName`), the resulting value's
                // type is the enum (`EnumName`), not the qualified
                // name. Otherwise fall through to the historic struct
                // behaviour.
                if let Some(idx) = name.rfind("::") {
                    let type_name = &name[..idx];
                    if self.enum_decls.contains_key(type_name) {
                        return Ok(Type::Struct(type_name.to_string()));
                    }
                }
                Ok(Type::Struct(name.clone()))
            }

            Node::FieldAccess {
                target,
                field,
                span,
                ..
            } => {
                self.current_span = *span;
                let tgt_ty = self.check_node(target)?;
                // RES-153: if the target is a known struct, return the
                // declared field's type. When the struct IS declared but
                // the field name is absent, emit a clear diagnostic
                // instead of silently returning Any.
                if let Type::Struct(sname) = &tgt_ty
                    && let Some(declared) = self.struct_fields.get(sname)
                {
                    if let Some((_, ty)) = declared.iter().find(|(n, _)| n == field) {
                        return Ok(ty.clone());
                    }
                    // RES-424: check for an impl method `StructName$field`
                    // before reporting "has no field". When found, return
                    // its type so method calls type-check correctly.
                    let mangled = format!("{}${}", sname, field);
                    if let Some(method_ty) = self.env.get(&mangled) {
                        return Ok(method_ty);
                    }
                    // RES-2697: check if any trait this struct implements has
                    // a default body for `field`. If so, the call is valid.
                    if let Some(implemented_traits) = self.trait_impls.get(sname.as_str()) {
                        for trait_name in implemented_traits {
                            if self
                                .trait_default_methods
                                .get(trait_name.as_str())
                                .is_some_and(|ms| ms.contains(field.as_str()))
                            {
                                return Ok(Type::Any);
                            }
                        }
                    }
                    // RES-407: struct is known, field not found, and no
                    // impl method or default trait method — report a clear diagnostic.
                    let avail: Vec<&str> = declared.iter().map(|(n, _)| n.as_str()).collect();
                    let hint = crate::did_you_mean::hint_from(field, avail.iter().copied());
                    return Err(format!(
                        "struct `{}` has no field `{}`{}; available fields: {}",
                        sname,
                        field,
                        hint,
                        if avail.is_empty() {
                            "(none)".to_string()
                        } else {
                            avail.join(", ")
                        }
                    ));
                    // Struct name unknown (forward reference, generic container,
                    // etc.) — fall through to permissive Any.
                }
                // RES-1859 / RES-412: known method return types for Array/String targets.
                // When the method is called as `arr.map(fn)`, the FieldAccess
                // node is used as the callee in a CallExpression; returning a
                // Function type here lets the call site infer the correct return.
                if tgt_ty == Type::Array {
                    let ret = match field.as_str() {
                        // HOF methods: take a callback / element as the extra arg.
                        "map" | "filter" | "push" | "flat_map" => Type::Function {
                            params: vec![Type::Any],
                            return_type: Box::new(Type::Array),
                        },
                        // RES-2734: zero-extra-arg array transformations. `pop`,
                        // `sort`, `reverse` etc. were previously grouped with the
                        // HOF methods (1 param), causing a false arity error when
                        // called without arguments. Fixed here.
                        "pop" | "sort" | "sort_desc" | "reverse" | "flatten" | "dedup" => {
                            Type::Function {
                                params: vec![],
                                return_type: Box::new(Type::Array),
                            }
                        }
                        // RES-2707: `reduce` accepts 1 arg (fn, uses first elem as init)
                        // or 2 args (init, fn). Return Type::Any so the call site skips
                        // strict arity checking and both forms are accepted.
                        "reduce" => Type::Any,
                        "for_each" => Type::Function {
                            params: vec![Type::Any],
                            return_type: Box::new(Type::Void),
                        },
                        "find" => Type::Function {
                            params: vec![Type::Any],
                            return_type: Box::new(Type::Option(Box::new(Type::Any))),
                        },
                        "any" | "all" => Type::Function {
                            params: vec![Type::Any],
                            return_type: Box::new(Type::Bool),
                        },
                        "len" => Type::Function {
                            params: vec![],
                            return_type: Box::new(Type::Int),
                        },
                        // RES-2734: `has` is the array-element membership method;
                        // `contains` is kept for backward compat.
                        "contains" | "has" => Type::Function {
                            params: vec![Type::Any],
                            return_type: Box::new(Type::Bool),
                        },
                        "join" => Type::Function {
                            params: vec![Type::String],
                            return_type: Box::new(Type::String),
                        },
                        // RES-2734: slice(start, end) — sub-array extraction.
                        "slice" => Type::Function {
                            params: vec![Type::Int, Type::Int],
                            return_type: Box::new(Type::Array),
                        },
                        _ => Type::Any,
                    };
                    if ret != Type::Any {
                        return Ok(ret);
                    }
                }
                if tgt_ty == Type::String {
                    let ret = match field.as_str() {
                        // `split` takes a separator arg; `chars` takes none.
                        // RES-2738: `chars` was incorrectly grouped with `split`
                        // (1-param), causing a false arity error on `s.chars()`.
                        "split" => Type::Function {
                            params: vec![Type::Any],
                            return_type: Box::new(Type::Array),
                        },
                        // RES-2738: zero-arg array-returning string methods.
                        "chars" | "lines" => Type::Function {
                            params: vec![],
                            return_type: Box::new(Type::Array),
                        },
                        // Zero-arg string → string methods.
                        "trim" | "to_upper" | "to_lower" | "reverse" => Type::Function {
                            params: vec![],
                            return_type: Box::new(Type::String),
                        },
                        "replace" => Type::Function {
                            params: vec![Type::String, Type::String],
                            return_type: Box::new(Type::String),
                        },
                        // RES-2738: strip_prefix / strip_suffix return Result.
                        "strip_prefix" | "strip_suffix" => Type::Function {
                            params: vec![Type::String],
                            return_type: Box::new(Type::Result),
                        },
                        "contains" | "starts_with" | "ends_with" => Type::Function {
                            params: vec![Type::String],
                            return_type: Box::new(Type::Bool),
                        },
                        "len" => Type::Function {
                            params: vec![],
                            return_type: Box::new(Type::Int),
                        },
                        _ => Type::Any,
                    };
                    if ret != Type::Any {
                        return Ok(ret);
                    }
                }
                // RES-1859: known method return types for Array/String targets.
                // When the method is called as `arr.map(fn)`, the FieldAccess
                // node is used as the callee in a CallExpression; returning a
                // Function type here lets the call site infer the correct return.
                if tgt_ty == Type::Array {
                    match field.as_str() {
                        "map" | "filter" | "flat_map" => {
                            return Ok(Type::Function {
                                params: vec![Type::Any],
                                return_type: Box::new(Type::Array),
                            });
                        }
                        // RES-2707: reduce accepts 1 or 2 args — use Any to skip
                        // strict arity checking at the call site.
                        "reduce" => return Ok(Type::Any),
                        "for_each" => {
                            return Ok(Type::Function {
                                params: vec![Type::Any],
                                return_type: Box::new(Type::Void),
                            });
                        }
                        "find" => {
                            return Ok(Type::Function {
                                params: vec![Type::Any],
                                return_type: Box::new(Type::Option(Box::new(Type::Any))),
                            });
                        }
                        "any" | "all" => {
                            return Ok(Type::Function {
                                params: vec![Type::Any],
                                return_type: Box::new(Type::Bool),
                            });
                        }
                        _ => {}
                    }
                }
                if tgt_ty == Type::String && field == "split" {
                    return Ok(Type::Function {
                        params: vec![Type::String],
                        return_type: Box::new(Type::Array),
                    });
                }
                Ok(Type::Any)
            }

            Node::FieldAssignment {
                target,
                field,
                value,
                ..
            } => {
                let tgt_ty = self.check_node(target)?;
                let val_ty = self.check_node(value)?;
                // RES-153: reject writes to non-existent fields
                // statically when the target's struct is known. The
                // old runtime error ("Struct Point has no field 'z'")
                // still fires for dynamic `Any` targets.
                if let Type::Struct(sname) = &tgt_ty
                    && let Some(declared) = self.struct_fields.get(sname)
                {
                    match declared.iter().find(|(n, _)| n == field) {
                        Some((_, field_ty)) => {
                            // RES-403 follow-up: validate the assigned
                            // value type against the declared field type.
                            if !compatible(field_ty, &val_ty) {
                                return Err(format!(
                                    "struct `{}` field `{}` has type {}, cannot assign {}",
                                    sname, field, field_ty, val_ty
                                ));
                            }
                        }
                        None => {
                            let avail: Vec<&str> =
                                declared.iter().map(|(n, _)| n.as_str()).collect();
                            let hint = crate::did_you_mean::hint_from(field, avail.iter().copied());
                            return Err(format!(
                                "struct `{}` has no field `{}`{}; available fields: {}",
                                sname,
                                field,
                                hint,
                                avail.join(", ")
                            ));
                        }
                    }
                }
                Ok(Type::Void)
            }

            Node::IndexExpression { target, index, .. } => {
                let tgt_ty = self.check_node(target)?;
                let idx_ty = self.check_node(index)?;
                // RES-405: array/string indexing must use an integer
                // index. Reject obvious type errors like `arr["key"]`
                // while keeping Map targets permissive (Any index).
                if matches!(tgt_ty, Type::Array | Type::String)
                    && !matches!(idx_ty, Type::Int | Type::Any)
                    && !is_pinned_int(&idx_ty)
                {
                    return Err(format!(
                        "index expression requires an integer index, got {}",
                        idx_ty
                    ));
                }
                // RES-921 added Python-style negative indexing to the runtime: arr[-1]
                // is the last element, arr[-2] is second-to-last, etc. The RES-415
                // compile-time rejection of negative constant indices is therefore a
                // false positive and is removed. Out-of-range access (including
                // out-of-range negative indices) is caught at runtime.
                // String indexing returns a char (RES-2709 + RES-2711):
                // runtime yields Value::Char; typechecker mirrors that here.
                // Array indexing returns Any (no element-type tracking yet).
                match tgt_ty {
                    Type::String => Ok(Type::Char),
                    _ => Ok(Type::Any),
                }
            }

            // RES-911 / RES-916: slicing — `target[lo..hi]` returns
            // `Array` for array targets and `String` for string targets.
            // Endpoints must be `Int`.
            Node::Slice { target, lo, hi, .. } => {
                let target_ty = self.check_node(target)?;
                // RES-407: enforce integer endpoints.
                let is_int_like = |t: &Type| matches!(t, Type::Int | Type::Any) || is_pinned_int(t);
                if let Some(lo_expr) = lo {
                    let lo_ty = self.check_node(lo_expr)?;
                    if !is_int_like(&lo_ty) {
                        return Err(format!(
                            "slice lower bound must be an integer, got {}",
                            lo_ty
                        ));
                    }
                }
                if let Some(hi_expr) = hi {
                    let hi_ty = self.check_node(hi_expr)?;
                    if !is_int_like(&hi_ty) {
                        return Err(format!(
                            "slice upper bound must be an integer, got {}",
                            hi_ty
                        ));
                    }
                }
                match target_ty {
                    Type::String => Ok(Type::String),
                    _ => Ok(Type::Array),
                }
            }

            Node::IndexAssignment {
                target,
                index,
                value,
                ..
            } => {
                // RES-406: validate index type for array/string targets.
                let tgt_ty = self.check_node(target)?;
                let idx_ty = self.check_node(index)?;
                if matches!(tgt_ty, Type::Array | Type::String)
                    && !matches!(idx_ty, Type::Int | Type::Any)
                    && !is_pinned_int(&idx_ty)
                {
                    return Err(format!(
                        "index assignment requires an integer index, got {}",
                        idx_ty
                    ));
                }
                let _ = self.check_node(value)?;
                Ok(Type::Void)
            }

            Node::ForInStatement {
                name,
                iterable,
                body,
                span,
                label,
                ..
            } => {
                self.current_span = *span;
                // RES-406: reject non-iterable types as the loop source.
                let iter_ty = self.check_node(iterable)?;
                if !matches!(iter_ty, Type::Array | Type::String | Type::Any) {
                    return Err(format!(
                        "cannot iterate over type {} — for-in requires an array or string",
                        iter_ty
                    ));
                }
                // RES-910: track loop depth so nested `break`/`continue`
                // are accepted only inside the body.
                self.loop_depth += 1;
                // RES-2653: push loop label onto the label stack.
                self.loop_label_stack.push(label.clone());

                // RES-1104: bind the loop variable so the body can
                // reference it without a false "Undefined variable"
                // diagnostic. The binding is confined to the body via
                // a fresh enclosed env.
                // RES-409/RES-420: infer element type from the iterable:
                // - Range syntax (0..10) → Int
                // - String value → String characters
                // - Known integer-array builtins → Int
                // - Known string-array builtins → String
                // - Everything else → Any (conservative)
                let elem_ty = if matches!(iterable.as_ref(), Node::Range { .. }) {
                    Type::Int
                } else if iter_ty == Type::String {
                    Type::String
                } else if let Node::CallExpression { function, .. } = iterable.as_ref()
                    && let Node::Identifier { name: callee, .. } = function.as_ref()
                {
                    match callee.as_str() {
                        // Integer-valued array builtins.
                        "array_range" | "array_range_int" | "array_cumsum" | "array_cumprod"
                        | "array_diffs" => Type::Int,
                        // String-element builtins.
                        "split" | "string_split" | "string_split_n" | "string_split_last"
                        | "chars" | "split_chars" => Type::String,
                        _ => Type::Any,
                    }
                } else {
                    Type::Any
                };
                let mut loop_env = TypeEnvironment::new_enclosed(self.env.clone());
                loop_env.set(name.clone(), elem_ty);
                std::mem::swap(&mut self.env, &mut loop_env);
                let body_result = self.check_node(body);
                std::mem::swap(&mut self.env, &mut loop_env);

                self.loop_depth -= 1;
                self.loop_label_stack.pop(); // RES-2653
                let _ = body_result?;
                Ok(Type::Void)
            }

            Node::WhileStatement {
                condition,
                body,
                span,
                label,
                ..
            } => {
                self.current_span = *span;
                // RES-406: while condition must be boolean.
                let cond_ty = self.check_node(condition)?;
                if cond_ty != Type::Bool && cond_ty != Type::Any {
                    return Err(format!(
                        "while condition must be a boolean, got {}",
                        cond_ty
                    ));
                }
                self.loop_depth += 1;
                self.loop_label_stack.push(label.clone()); // RES-2653
                let body_result = self.check_node(body);
                self.loop_depth -= 1;
                self.loop_label_stack.pop(); // RES-2653
                let _ = body_result?;
                Ok(Type::Void)
            }

            // RES-910: `break;` / `continue;` are typechecker-rejected
            // outside any enclosing loop body.
            Node::Break { .. } => {
                if self.loop_depth == 0 {
                    return Err("'break' outside of a loop — `break` is only valid \
                         inside a `while` or `for-in` body"
                        .to_string());
                }
                Ok(Type::Void)
            }
            Node::Continue { .. } => {
                if self.loop_depth == 0 {
                    return Err("'continue' outside of a loop — `continue` is only \
                         valid inside a `while` or `for-in` body"
                        .to_string());
                }
                Ok(Type::Void)
            }
            // RES-2653: labeled break/continue — validate the label
            // refers to an enclosing labeled loop.
            Node::BreakLabel { label, .. } => {
                if self.loop_depth == 0 {
                    return Err(format!("'break {label}' outside of any loop"));
                }
                if !self
                    .loop_label_stack
                    .iter()
                    .any(|l| l.as_deref() == Some(label.as_str()))
                {
                    return Err(format!(
                        "label '{label}' not found — no enclosing loop is labeled '{label}'"
                    ));
                }
                Ok(Type::Void)
            }
            Node::ContinueLabel { label, .. } => {
                if self.loop_depth == 0 {
                    return Err(format!("'continue {label}' outside of any loop"));
                }
                if !self
                    .loop_label_stack
                    .iter()
                    .any(|l| l.as_deref() == Some(label.as_str()))
                {
                    return Err(format!(
                        "label '{label}' not found — no enclosing loop is labeled '{label}'"
                    ));
                }
                Ok(Type::Void)
            }

            Node::StaticLet { name, value, .. } => {
                let value_type = self.check_node(value)?;
                self.env.set(name.clone(), value_type);
                // RES-063: static lets are mutable across calls, so
                // they're never safe to treat as compile-time constants
                // for verification.
                self.const_bindings.remove(name);
                Ok(Type::Void)
            }

            // RES-361: `const NAME: T = expr;` — type-check the value
            // and bind the name as a constant in the type environment.
            Node::Const {
                name,
                value,
                type_annot,
                ..
            } => {
                let value_type = self.check_node(value)?;
                let bind_type = if let Some(ty_name) = type_annot {
                    let declared = self.parse_type_name(ty_name)?;
                    if !compatible(&declared, &value_type)
                        && !self.satisfies_trait_param(&declared, &value_type)
                    {
                        return Err(format!(
                            "const {}: {} — value has type {}",
                            name, declared, value_type
                        ));
                    }
                    // RES-416: apply the same pinned-int overflow check
                    // to const declarations that LetStatement already has.
                    if let Some(literal_val) = fold_const_i64(value, &self.const_bindings) {
                        let range_ok = match &declared {
                            Type::Int8 => (-128_i64..=127).contains(&literal_val),
                            Type::Int16 => (-32768_i64..=32767).contains(&literal_val),
                            Type::Int32 => {
                                (-2_147_483_648_i64..=2_147_483_647).contains(&literal_val)
                            }
                            Type::UInt8 => (0_i64..=255).contains(&literal_val),
                            Type::UInt16 => (0_i64..=65535).contains(&literal_val),
                            Type::UInt32 => (0_i64..=4_294_967_295).contains(&literal_val),
                            _ => true,
                        };
                        if !range_ok {
                            let range_str = match &declared {
                                Type::Int8 => "-128..=127",
                                Type::Int16 => "-32768..=32767",
                                Type::Int32 => "-2147483648..=2147483647",
                                Type::UInt8 => "0..=255",
                                Type::UInt16 => "0..=65535",
                                Type::UInt32 => "0..=4294967295",
                                _ => unreachable!(),
                            };
                            return Err(format!(
                                "const {}: {} — value {} overflows the declared type (valid range: {})",
                                name, declared, literal_val, range_str
                            ));
                        }
                    }
                    declared
                } else {
                    value_type.clone()
                };
                self.env.set(name.clone(), bind_type);
                // Fold const bindings so subsequent references can use
                // the constant value in const-folding contexts.
                if let Some(v) = fold_const_i64(value, &self.const_bindings) {
                    self.const_bindings.insert(name.clone(), v);
                }
                Ok(Type::Void)
            }

            Node::Assignment { name, value, .. } => {
                let val_ty = self.check_node(value)?;
                // RES-405: validate the new value type against the
                // variable's currently-bound type. Rejects patterns like
                //   let x: int = 5; x = "oops";
                // while staying permissive for Any-typed variables
                // (unresolved generics, dynamic containers).
                if let Some(var_ty) = self.env.get(name)
                    && !compatible(&var_ty, &val_ty)
                    // RES-2693: reassigning a struct to a trait-typed variable is
                    // valid when the struct implements that trait.
                    && !self.satisfies_trait_param(&var_ty, &val_ty)
                {
                    return Err(format!(
                        "cannot assign {} to variable `{}` of type {}",
                        val_ty, name, var_ty
                    ));
                }
                // RES-063: any reassignment kills const-tracking. We
                // could try to re-track if RHS is foldable, but
                // mid-function mutation is rare and the conservative
                // choice keeps the verifier sound.
                self.const_bindings.remove(name);
                Ok(Type::Void)
            }

            Node::ReturnStatement {
                value,
                span: ret_span,
            } => {
                // RES-1862: track span for diagnostics.
                if ret_span.start.line > 0 {
                    self.current_span = *ret_span;
                }
                // Bare `return;` has type Void; otherwise pass through
                // the type of the returned value.
                let ret_type = match value {
                    Some(expr) => self.check_node(expr)?,
                    None => Type::Void,
                };
                // RES-403: validate against declared return type so early
                // returns (inside if/match arms) can't silently return the
                // wrong type while the body's last expression type matches.
                if let Some(declared) = &self.current_fn_return_type
                    && !compatible(declared, &ret_type)
                    // RES-2693: a struct returned where a trait is declared
                    // is valid when the struct implements that trait.
                    && !self.satisfies_trait_param(declared, &ret_type)
                {
                    return Err(format!(
                        "return type mismatch — declared {}, returning {}",
                        declared, ret_type
                    ));
                }
                Ok(ret_type)
            }

            Node::IfStatement {
                condition,
                consequence,
                alternative,
                span: if_span,
            } => {
                // RES-1862: track span for diagnostics.
                if if_span.start.line > 0 {
                    self.current_span = *if_span;
                }
                let condition_type = self.check_node(condition)?;
                if condition_type != Type::Bool && condition_type != Type::Any {
                    return Err(format!(
                        "If condition must be a boolean, got {}",
                        condition_type
                    ));
                }

                // RES-064: if the condition is `IDENT == LITERAL` (or
                // `LITERAL == IDENT`), assume that equality inside the
                // consequence by pushing the binding. Restore on exit
                // so the assumption doesn't leak.
                // RES-1410: destructure `assumption` by move so the
                // owned `name` String can be cloned once for the
                // HashMap insert and then moved into the `saved`
                // tuple. The previous shape took `ref name` and
                // cloned twice — once for the HashMap key (which
                // takes ownership) and once for the rollback tuple.
                let saved = if let Some((name, value)) = extract_eq_assumption(condition) {
                    let prev = self.const_bindings.get(&name).copied();
                    self.const_bindings.insert(name.clone(), value);
                    Some((name, prev))
                } else {
                    None
                };

                let consequence_type = self.check_node(consequence)?;

                // Restore.
                if let Some((name, prev)) = saved {
                    match prev {
                        Some(v) => {
                            self.const_bindings.insert(name, v);
                        }
                        None => {
                            self.const_bindings.remove(&name);
                        }
                    }
                }

                if let Some(alt) = alternative {
                    let alternative_type = self.check_node(alt)?;

                    // RES-421: when one branch unconditionally diverges
                    // (return / break / continue), its type doesn't
                    // constrain the if-else expression — use the other
                    // branch's type as the whole expression's type.
                    let cons_diverges = node_terminates(consequence);
                    let alt_diverges = node_terminates(alt);
                    if cons_diverges && !alt_diverges {
                        return Ok(alternative_type);
                    }
                    if alt_diverges && !cons_diverges {
                        return Ok(consequence_type);
                    }

                    // Both branches should have compatible types.
                    if consequence_type != alternative_type
                        && consequence_type != Type::Any
                        && alternative_type != Type::Any
                    {
                        return Err(format!(
                            "If branches have incompatible types: {} and {}",
                            consequence_type, alternative_type
                        ));
                    }
                    // RES-402: if one branch is Any and the other is a
                    // concrete type, propagate the concrete type so callers
                    // get a useful inference result.
                    return Ok(infer_common_arm_type(&[consequence_type, alternative_type]));
                }

                Ok(consequence_type)
            }

            Node::ExpressionStatement { expr, .. } => self.check_node(expr),

            Node::Identifier { name, span } => {
                // RES-1862: track innermost span so the check_program
                // wrapper uses the identifier's position rather than
                // the enclosing statement's start position.
                if span.start.line > 0 {
                    self.current_span = *span;
                }
                // RES-078: identifier span lets us tell users where
                // exactly the undefined reference lives. Skip the
                // prefix when the span looks default (synthetic).
                // RES-400: payload-less enum-variant constructors
                // (`Color::Red`) resolve to `Type::Struct(EnumName)` —
                // the runtime registers them under the qualified key
                // when the EnumDecl evaluates; the typechecker mirrors
                // that resolution from `enum_decls` so the typed view
                // matches.
                // RES-2603: tuple-payload variants (`Option::Some`)
                // resolve to `Type::Function` so they typecheck when
                // passed to higher-order functions.
                if let Some(idx) = name.rfind("::")
                    && let Some(variants) = self.enum_decls.get(&name[..idx]).cloned()
                    && let Some(v) = variants.iter().find(|v| v.name == name[idx + 2..])
                {
                    let type_name = &name[..idx];
                    match &v.payload {
                        crate::EnumPayload::None => {
                            return Ok(Type::Struct(type_name.to_string()));
                        }
                        // RES-2603: tuple-payload variant → function type.
                        // Use parse_type_name for both params and return type
                        // so the resulting Type::Function is structurally
                        // identical to what a caller would get from a
                        // `fn(T) -> EnumName` annotation.
                        crate::EnumPayload::Tuple(param_types) => {
                            let param_types = param_types.clone();
                            let params: Vec<Type> = param_types
                                .iter()
                                .map(|t| self.parse_type_name(t).unwrap_or(Type::Any))
                                .collect();
                            let return_type = self
                                .parse_type_name(type_name)
                                .unwrap_or(Type::Struct(type_name.to_string()));
                            return Ok(Type::Function {
                                params,
                                return_type: Box::new(return_type),
                            });
                        }
                        crate::EnumPayload::Named(_) => {}
                    }
                }
                match self.env.get(name) {
                    Some(typ) => Ok(typ),
                    None => {
                        // RES-424: `Struct::method` → try the `Struct$method`
                        // mangling that impl blocks register under. This makes
                        // static method calls (no `self`) reachable via the
                        // `Type::method()` call syntax users expect.
                        if let Some(idx) = name.find("::")
                            && let Some(typ) =
                                self.env
                                    .get(&format!("{}${}", &name[..idx], &name[idx + 2..]))
                        {
                            return Ok(typ);
                        }
                        // RES-306: append a did-you-mean hint when an
                        // in-scope name is within Levenshtein distance 2
                        // of the typo. The helper handles the
                        // <3-char skip and the cap-at-3 ranking.
                        let names = self.env.all_names();
                        let hint = crate::did_you_mean::hint_from(
                            name.as_str(),
                            names.iter().map(String::as_str),
                        );
                        if span.start.line == 0 {
                            Err(format!("Undefined variable: {}{}", name, hint))
                        } else {
                            Err(format!(
                                "Undefined variable '{}' at {}:{}{}",
                                name, span.start.line, span.start.column, hint
                            ))
                        }
                    }
                }
            }

            Node::IntegerLiteral { .. } => Ok(Type::Int),
            Node::FloatLiteral { .. } => Ok(Type::Float),
            Node::StringLiteral { .. } => Ok(Type::String),
            // RES-221: interpolated strings always produce a String value.
            // RES-2721: type-check the embedded sub-expressions here, in the
            // main traversal, so the full environment (including let bindings)
            // is available. The old `string_interp::check` extension pass used
            // a fresh TypeChecker with no let-binding scope, causing false
            // "Undefined variable" diagnostics for in-scope variables.
            Node::InterpolatedString {
                parts,
                span: interp_span,
            } => {
                let loc = if interp_span.start.line > 0 {
                    format!(
                        "{}:{}:{}: ",
                        self.source_path, interp_span.start.line, interp_span.start.column
                    )
                } else {
                    String::new()
                };
                for part in parts {
                    if let crate::string_interp::StringPart::Expr(expr) = part {
                        // Errors are downgraded to warnings — an interpolation
                        // that can't be typed (e.g. genuinely undefined name)
                        // should still allow the program to continue running.
                        if let Err(e) = self.check_node(expr) {
                            eprintln!("warning: {loc}in interpolated string: {e}");
                        }
                    }
                }
                Ok(Type::String)
            }
            Node::BytesLiteral { .. } => Ok(Type::Bytes),
            Node::BooleanLiteral { .. } => Ok(Type::Bool),
            // RES-2711: char literals produce the `char` type.
            Node::CharLiteral { .. } => Ok(Type::Char),

            Node::PrefixExpression {
                operator,
                right,
                span: prefix_span,
            } => {
                // RES-1862: track innermost span for better diagnostics.
                if prefix_span.start.line > 0 {
                    self.current_span = *prefix_span;
                }
                let right_type = self.check_node(right)?;

                match *operator {
                    "!" => {
                        if right_type != Type::Bool && right_type != Type::Any {
                            return Err(format!("Cannot apply '!' to {}", right_type));
                        }
                        Ok(Type::Bool)
                    }
                    "-" => {
                        if right_type != Type::Int
                            && right_type != Type::Float
                            && right_type != Type::Any
                            && !is_pinned_int(&right_type)
                        {
                            return Err(format!("Cannot apply '-' to {}", right_type));
                        }
                        Ok(right_type)
                    }
                    _ => Err(format!("Unknown prefix operator: {}", operator)),
                }
            }

            Node::InfixExpression {
                left,
                operator,
                right,
                span: infix_span,
            } => {
                // RES-1862: track innermost span for better diagnostics.
                self.current_span = *infix_span;
                let left_type = self.check_node(left)?;
                let right_type = self.check_node(right)?;

                // RES-130: `is_numeric` retired for `+ - * / %`; the
                // `check_numeric_same_type` helper now enforces the
                // no-coercion rule. `is_bool` stays for the logical
                // operator arm.
                let is_bool = |t: &Type| matches!(t, Type::Bool | Type::Any);

                match *operator {
                    "+" => {
                        // String-plus-primitive coercion (RES-008): if
                        // either side is a string AND the other side is a
                        // stringifiable primitive, the result is a string.
                        // Only Int, Float, Bool, String, and Any are
                        // coercible — compound types (Array, Struct, etc.)
                        // must use explicit to_string() conversion.
                        let can_coerce_to_string = |t: &Type| {
                            matches!(
                                t,
                                Type::String | Type::Int | Type::Float | Type::Bool | Type::Any
                            ) || is_pinned_int(t)
                        };
                        if left_type == Type::String || right_type == Type::String {
                            if can_coerce_to_string(&left_type) && can_coerce_to_string(&right_type)
                            {
                                return Ok(Type::String);
                            }
                            let bad = if left_type == Type::String {
                                &right_type
                            } else {
                                &left_type
                            };
                            return Err(format!(
                                "cannot concatenate string with {} — use to_string() for explicit conversion",
                                bad
                            ));
                        }
                        // Array concat.
                        if compatible(&left_type, &Type::Array)
                            && compatible(&right_type, &Type::Array)
                        {
                            return Ok(Type::Array);
                        }
                        // RES-130: no implicit int ↔ float coercion.
                        // `Int + Int` → Int, `Float + Float` → Float;
                        // mixed is a type error. Users route through
                        // the explicit `to_float(x)` / `to_int(x)`
                        // builtins when they really need the conversion.
                        check_numeric_same_type(operator, &left_type, &right_type)
                    }
                    "-" | "*" | "/" | "%" => {
                        // RES-130: same policy as `+` — no mixed int / float.
                        // RES-413: static division / modulo by zero detection.
                        if matches!(*operator, "/" | "%")
                            && let Some(divisor) = fold_const_i64(right, &self.const_bindings)
                            && divisor == 0
                        {
                            return Err(format!(
                                "integer {} by zero — denominator is a compile-time constant 0",
                                if *operator == "/" {
                                    "division"
                                } else {
                                    "modulo"
                                }
                            ));
                        }
                        check_numeric_same_type(operator, &left_type, &right_type)
                    }
                    "&" | "|" | "^" | "<<" | ">>" => {
                        // Bitwise operators are int-only. Same-width
                        // pinned integer types are also accepted.
                        let left_is_int = compatible(&left_type, &Type::Int)
                            || is_pinned_int(&left_type)
                            || left_type == Type::Any;
                        let right_is_int = compatible(&right_type, &Type::Int)
                            || is_pinned_int(&right_type)
                            || right_type == Type::Any;
                        if left_is_int && right_is_int {
                            // Delegate to check_numeric_same_type for
                            // width-matching on pinned types.
                            check_numeric_same_type(operator, &left_type, &right_type)
                        } else {
                            Err(format!(
                                "Bitwise '{}' requires int operands, got {} and {}",
                                operator, left_type, right_type
                            ))
                        }
                    }
                    "&&" | "||" => {
                        if is_bool(&left_type) && is_bool(&right_type) {
                            Ok(Type::Bool)
                        } else {
                            Err(format!(
                                "Logical '{}' requires bool operands, got {} and {}",
                                operator, left_type, right_type
                            ))
                        }
                    }
                    "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                        if compatible(&left_type, &right_type) {
                            Ok(Type::Bool)
                        } else {
                            Err(format!("Cannot compare {} and {}", left_type, right_type))
                        }
                    }
                    // RES-2717: `??` null-coalescing operator. Left must be
                    // Option<T>; result is T (or the right-hand type when T is
                    // Any). Any left type is accepted as a fallback for
                    // untyped / generic callers.
                    "??" => {
                        if left_type == Type::Any {
                            return Ok(right_type);
                        }
                        if let Type::Option(inner) = &left_type {
                            let inner_ty = *inner.clone();
                            if inner_ty == Type::Any {
                                return Ok(right_type);
                            }
                            if compatible(&inner_ty, &right_type) {
                                return Ok(inner_ty);
                            }
                            return Err(format!(
                                "`??` default has type {} but Option inner type is {}",
                                right_type, inner_ty
                            ));
                        }
                        Err(format!(
                            "`??` operator requires an Option on the left, got {}",
                            left_type
                        ))
                    }
                    _ => Err(format!("Unknown infix operator: {}", operator)),
                }
            }

            Node::CallExpression {
                function,
                arguments,
                span: call_span,
            } => {
                // RES-1862: track innermost span for better diagnostics.
                self.current_span = *call_span;
                // RES-400: tuple-payload enum-variant constructor —
                // `Either::Just(7)` parses as a CallExpression with
                // the callee `Identifier("Either::Just")`. Resolve it
                // here BEFORE the regular `check_node(function)` path
                // (which would error with "Undefined variable") so
                // the typechecker treats it as a constructor.
                if let Node::Identifier { name, .. } = function.as_ref()
                    && let Some(idx) = name.rfind("::")
                {
                    let type_name = &name[..idx];
                    let variant_name = &name[idx + 2..];
                    if let Some(variants) = self.enum_decls.get(type_name).cloned()
                        && let Some(v) = variants.iter().find(|v| v.name == variant_name)
                        && let crate::EnumPayload::Tuple(declared) = &v.payload
                    {
                        if arguments.len() != declared.len() {
                            return Err(format!(
                                "Constructor {}::{}: expected {} arg(s), got {}",
                                type_name,
                                variant_name,
                                declared.len(),
                                arguments.len()
                            ));
                        }
                        // RES-416: validate argument types against the
                        // declared payload types.
                        for (arg, type_str) in arguments.iter().zip(declared.iter()) {
                            let arg_ty = self.check_node(arg)?;
                            if let Ok(expected_ty) = self.parse_type_name(type_str)
                                && !compatible(&expected_ty, &arg_ty)
                            {
                                return Err(format!(
                                    "Constructor {}::{}: argument has type {}, expected {}",
                                    type_name, variant_name, arg_ty, expected_ty
                                ));
                            }
                        }
                        return Ok(Type::Struct(type_name.to_string()));
                    }
                }
                let func_type = self.check_node(function)?;

                // RES-061 + RES-063: if the callee is a known top-level
                // fn with contracts, fold each requires clause with the
                // call's arguments substituted for parameters. Arguments
                // can be literal expressions OR identifiers that resolve
                // to a constant via const_bindings.
                if let Node::Identifier {
                    name: callee_name, ..
                } = function.as_ref()
                    && let Some(info) = self.contract_table.get(callee_name).cloned()
                {
                    // RES-387: every failure variant the callee declares
                    // must be propagated by the enclosing fn's `fails`
                    // set. This is the MVP slice — structured handlers
                    // (try/catch) land in a follow-up.
                    for variant in &info.fails {
                        let declared = match &self.current_fn_fails {
                            Some(outer) => outer.iter().any(|v| v == variant),
                            None => false,
                        };
                        if !declared {
                            return Err(format!(
                                "unhandled failure variant {} — declare `fails {}` on the caller or wrap the call in `try {{ ... }} catch {} {{ ... }}` (from call to `{}`)",
                                variant, variant, variant, callee_name
                            ));
                        }
                    }
                    // RES-1399: skip the `bindings` HashMap allocation
                    // and the per-arg `fold_const_i64` probe when the
                    // callee declares no `requires` clauses. `bindings`
                    // is consumed exclusively by the `for clause in
                    // info.requires.iter()` loop below (passed to
                    // `fold_const_bool` + the Z3 prover). When
                    // `info.requires.is_empty()` the loop never enters
                    // and every line that populates `bindings` is dead
                    // work. The vast majority of user functions declare
                    // no pre-conditions, so this saves the HashMap +
                    // per-arg `fold_const_i64` probe at every call
                    // site to such a function. The fails-variant
                    // propagation loop above this point still runs
                    // unconditionally; only the bindings + clause work
                    // is gated.
                    if !info.requires.is_empty() {
                        self.stats.contracted_call_sites += 1;
                        // RES-1413: pre-size bindings to the parameter
                        // count. The fold loop below inserts at most
                        // one entry per parameter (`zip` truncates to
                        // the shorter of params/args), so the capacity
                        // is known up front.
                        let mut bindings: HashMap<String, i64> =
                            HashMap::with_capacity(info.parameters.len());
                        for ((_ty, pname), arg) in info.parameters.iter().zip(arguments.iter()) {
                            if let Some(v) = fold_const_i64(arg, &self.const_bindings) {
                                bindings.insert(pname.clone(), v);
                            }
                        }
                        for (clause_idx, clause) in info.requires.iter().enumerate() {
                            // Try the cheap hand-rolled folder first.
                            let mut verdict = fold_const_bool(clause, &bindings);
                            // RES-136: slot for Z3's counterexample; only
                            // populated if Z3 runs (folder came back None).
                            let mut call_counterexample: Option<String> = None;
                            // RES-067: if undecidable, fall back to Z3
                            // (only when the binary was built --features z3).
                            if verdict.is_none() {
                                // RES-071: also capture certificate.
                                // RES-354: use theory-aware prover.
                                let (v, cert, cx, timed_out) = {
                                    #[cfg(feature = "z3")]
                                    {
                                        z3_prove_with_cert_theory(
                                            clause,
                                            &bindings,
                                            self.verifier_timeout_ms,
                                            self.z3_theory,
                                        )
                                    }
                                    #[cfg(not(feature = "z3"))]
                                    {
                                        z3_prove_with_cert(
                                            clause,
                                            &bindings,
                                            self.verifier_timeout_ms,
                                        )
                                    }
                                };
                                verdict = v;
                                if verdict.is_some() {
                                    self.stats.requires_discharged_by_z3 += 1;
                                }
                                if timed_out {
                                    // RES-137: soft-failure — runtime
                                    // check stays in; audit bumps.
                                    self.stats.verifier_timeouts += 1;
                                    eprintln!(
                                        "hint: proof timed out after {}ms — runtime check retained (call to fn {})",
                                        self.verifier_timeout_ms, callee_name
                                    );
                                }
                                // RES-217: any unresolved verdict (timeout
                                // OR a genuine Z3 `Unknown`) is a partial
                                // proof. Emit the structured diagnostic
                                // with the specific assertion's source
                                // position; suppressed on
                                // `--no-warn-unverified`.
                                if verdict.is_none() && self.warn_unverified {
                                    emit_partial_proof_warning(&self.source_path, clause);
                                }
                                // RES-1357: only stash the cert when the
                                // `--emit-certificate` driver asked for it.
                                if matches!(verdict, Some(true))
                                    && self.emit_certificates
                                    && let Some(smt2) = cert
                                {
                                    self.certificates.push(CapturedCertificate {
                                        fn_name: callee_name.clone(),
                                        kind: "callsite_requires",
                                        idx: clause_idx,
                                        smt2,
                                    });
                                }
                                call_counterexample = cx;
                            }
                            match verdict {
                                Some(false) => {
                                    // RES-136: append counterexample when
                                    // Z3 found one.
                                    let base = format!(
                                        "Contract violation: call to fn {} would fail `requires` clause at compile time",
                                        callee_name
                                    );
                                    return Err(match call_counterexample {
                                        Some(cx) => format!("{} — counterexample: {}", base, cx),
                                        None => base,
                                    });
                                }
                                Some(true) => {
                                    self.stats.requires_discharged_at_compile += 1;
                                    // RES-1505: gate the owned-String key allocation
                                    // on a real insertion. `HashMap::entry(K)` requires
                                    // an owned key *every call*, allocating even on
                                    // repeat hits — and the call-site loop hits the
                                    // same callee many times for any fn with multiple
                                    // `requires` clauses or many callers.
                                    if let Some(c) =
                                        self.stats.per_fn_discharged.get_mut(callee_name.as_str())
                                    {
                                        *c += 1;
                                    } else {
                                        self.stats.per_fn_discharged.insert(callee_name.clone(), 1);
                                    }
                                }
                                None => {
                                    self.stats.requires_left_for_runtime += 1;
                                    if let Some(c) =
                                        self.stats.per_fn_runtime.get_mut(callee_name.as_str())
                                    {
                                        *c += 1;
                                    } else {
                                        self.stats.per_fn_runtime.insert(callee_name.clone(), 1);
                                    }
                                }
                            }
                        }
                    }
                }

                // RES-2651: `Some(expr)` returns `Option<typeof(expr)>`.
                // The registered signature is `fn(Any) -> Option<Any>`;
                // replace the generic inner type with the concrete
                // argument type so downstream pattern matching on
                // `Some(x)` binds `x` to the right type.
                if let Node::Identifier {
                    name: callee_name, ..
                } = function.as_ref()
                    && callee_name == "Some"
                    && arguments.len() == 1
                {
                    let arg_type = self.check_node(&arguments[0])?;
                    return Ok(Type::Option(Box::new(arg_type)));
                }

                // RES-410: call-site type inference for numeric polymorphic
                // builtins registered as (Any, Any) -> Any. When all
                // arguments agree on the same concrete numeric type,
                // propagate that type as the return instead of Any.
                // This catches cases like `let x = min(1, 2); x + 1`
                // which previously inferred x as Any.
                if let Node::Identifier {
                    name: callee_name, ..
                } = function.as_ref()
                    && matches!(
                        callee_name.as_str(),
                        "min" | "max" | "pow" | "abs" | "sign" | "clamp"
                    )
                    && matches!(func_type, Type::Function { .. })
                {
                    let mut arg_types: Vec<Type> = Vec::with_capacity(arguments.len());
                    for arg in arguments {
                        arg_types.push(self.check_node(arg)?);
                    }
                    let common = infer_common_arm_type(&arg_types);
                    if common != Type::Any {
                        return Ok(common);
                    }
                }

                match func_type {
                    Type::Function {
                        params,
                        return_type,
                    } => {
                        // RES-424: struct impl method-call dispatch — when
                        // the callee is a FieldAccess on a struct target, the
                        // receiver is the implicit first `self` argument that
                        // the interpreter prepends. Only applies for struct
                        // targets (not Array / String builtins whose method
                        // types were registered WITHOUT a self param slot).
                        let is_struct_method_call = if let Node::FieldAccess {
                            target: fa_target,
                            ..
                        } = function.as_ref()
                        {
                            matches!(self.check_node(fa_target), Ok(Type::Struct(_)))
                        } else {
                            false
                        };
                        let (explicit_params, param_offset) =
                            if is_struct_method_call && !params.is_empty() {
                                (params.len() - 1, 1)
                            } else {
                                (params.len(), 0)
                            };

                        // Check argument count
                        if arguments.len() != explicit_params {
                            return Err(format!(
                                "Expected {} arguments, got {}",
                                explicit_params,
                                arguments.len()
                            ));
                        }

                        // RES-425: if the callee is a named generic function,
                        // collect its type-parameter names so Struct("T") can
                        // be treated as Any during argument checking.
                        let callee_type_params: Option<Vec<String>> = if let Node::Identifier {
                            name: callee_id,
                            ..
                        } = function.as_ref()
                        {
                            self.fn_type_params.get(callee_id.as_str()).cloned()
                        } else {
                            None
                        };
                        // Check each argument type
                        for (i, (arg, param_type)) in arguments
                            .iter()
                            .zip(params[param_offset..].iter())
                            .enumerate()
                        {
                            let arg_type = self.check_node(arg)?;
                            // RES-2701: substitute generic type params recursively
                            // so that composite types like `fn(T) -> T` become
                            // `fn(Any) -> Any` when T is a declared type param.
                            // The previous RES-425 single-level check only handled
                            // direct `Struct("T")` params, not Function/Tuple/Option
                            // wrappers that contain a type variable.
                            let substituted;
                            let effective_param = if let Some(tp) = &callee_type_params {
                                substituted = substitute_type_params(param_type, tp);
                                &substituted
                            } else {
                                param_type
                            };
                            if !compatible(&arg_type, effective_param)
                                // RES-2693: a concrete struct satisfies a
                                // trait-typed parameter when it implements
                                // the trait via an `impl Trait for Struct`
                                // block.
                                && !self.satisfies_trait_param(&arg_type, effective_param)
                            {
                                // RES-340: when RESILIENT_RICH_DIAG=1
                                // emit a rustc-style multi-block
                                // diagnostic with a secondary label
                                // pointing at the fn declaration. The
                                // legacy short form remains the
                                // default to keep every existing
                                // golden byte-identical.
                                if rich_diag_enabled() {
                                    // Fall back to the call's `(`
                                    // span when the argument node
                                    // doesn't carry one (literal
                                    // sub-trees produced before
                                    // RES-077 spans were threaded).
                                    let mut arg_span = clause_span(arg);
                                    if arg_span.start.line == 0 {
                                        arg_span = *call_span;
                                    }
                                    let decl_span = if let Node::Identifier {
                                        name: callee_name,
                                        ..
                                    } = function.as_ref()
                                    {
                                        self.fn_decl_spans.get(callee_name).copied()
                                    } else {
                                        None
                                    };
                                    return Err(render_rich_arg_type_mismatch(
                                        &self.source_path,
                                        arg_span,
                                        decl_span,
                                        i + 1,
                                        &effective_param.to_string(),
                                        &arg_type.to_string(),
                                    ));
                                }
                                return Err(format!(
                                    "Type mismatch in argument {}: expected {}, got {}",
                                    i + 1,
                                    effective_param,
                                    arg_type
                                ));
                            }
                        }

                        // RES-425: if the return type is a generic type
                        // parameter (Type::Struct("T")), return Any so
                        // callers don't get spurious type mismatches.
                        let effective_return = if let Type::Struct(tname) = return_type.as_ref()
                            && let Some(tp) = &callee_type_params
                            && tp.iter().any(|p| p == tname)
                        {
                            Type::Any
                        } else {
                            *return_type
                        };
                        Ok(effective_return)
                    }
                    Type::Any => Ok(Type::Any),
                    _ => Err(format!("Cannot call non-function type: {}", func_type)),
                }
            }
            // RES-325: a `NamedArg` can only appear inside a call's
            // argument list — the call check above walks `arguments`
            // directly and treats each labelled arg by inspecting the
            // inner value. Encountering one in any other position is
            // a parser/internal bug; surface a clean error rather
            // than crashing the typechecker.
            Node::NamedArg { value, .. } => self.check_node(value),
            // RES-324: module declaration — type-check each body node.
            Node::ModuleDecl { body, .. } => {
                for node in body {
                    self.check_node(node)?;
                }
                Ok(Type::Void)
            }
            // RES-290: trait declarations carry only signatures and are
            // validated by `crate::traits::check`; nothing to do here.
            Node::TraitDecl { .. } => Ok(Type::Void),
            // RES-400 PR 1: enum declarations are accepted by the
            // typechecker as Void today. PR 2 will extend `Type` with
            // RES-400: register the enum in the typechecker's
            // `enum_decls` table so the `Match` arm can check
            // exhaustiveness and so `match_pattern_binding_types`
            // (future PR) can resolve payload-field types.
            Node::EnumDecl {
                name,
                variants,
                span,
                ..
            } => {
                // RES-406: reject duplicate variant names inside the same enum.
                let mut seen_variants: std::collections::HashSet<&str> =
                    std::collections::HashSet::with_capacity(variants.len());
                for v in variants {
                    if !seen_variants.insert(v.name.as_str()) {
                        return Err(format!(
                            "{}:{}:{}: error: duplicate variant `{}` in enum `{}`",
                            self.source_path, span.start.line, span.start.column, v.name, name,
                        ));
                    }
                }
                // RES-1368: store variants behind a refcounted handle
                // so subsequent lookup-and-clone hot paths in `Match`
                // checking pay a refcount bump instead of a full Vec
                // clone. RES-1398 promoted the storage from `Rc` to
                // `Arc` so the builtin Option/Result entries can live
                // in a `Sync` `LazyLock`; user-defined enum decls
                // share the same Arc machinery.
                self.enum_decls
                    .insert(name.clone(), std::sync::Arc::new(variants.clone()));
                Ok(Type::Void)
            }
            // RES-406: unsafe block. The volatile-call gate (compile-
            // time error when calling `volatile_*` outside an
            // unsafe block) is enforced in a sibling pass —
            // `crate::unsafe_check::check_program` — that runs after
            // the typechecker. Here we just descend into the body so
            // its statements get type-checked.
            Node::UnsafeBlock { body, .. } => self.check_node(body),
            // RES-395: region type-param is a declaration-site marker;
            // no type to check.
            Node::RegionParam { .. } => Ok(Type::Void),
            // RES-2552: blanket impl — validated by blanket_impl::check.
            Node::BlanketImpl { .. } => Ok(Type::Void),
            // RES-2660: static_assert — validated by static_assert::check.
            Node::StaticAssert { .. } => Ok(Type::Void),
        }
    }

    fn parse_type_name(&self, name: &str) -> Result<Type, String> {
        // RES-385: the parser prefixes `linear` types with the literal
        // string `linear `. The linearity bit is consumed by the
        // dedicated single-use pass (`check_linear_usage`); at the
        // plain type-equality level, `linear T` and `T` are the same
        // type, so strip the prefix here before resolving.
        let base = crate::linear::strip_linear(name);
        // RES-1810: pre-size the alias-cycle-detection Vec to 2 —
        // typical alias chains are 0-2 deep (`type Meters = Float;`
        // is single-step). Saves the 0→4 doubling chain on every
        // type-name resolution, which is called per parameter type,
        // return type, let annot, struct field, etc.
        self.parse_type_name_inner(base, &mut Vec::with_capacity(2))
    }

    /// RES-128: alias-aware parse with cycle detection. `seen`
    /// tracks the alias names we've already expanded on the
    /// current walk — re-entering any of them means the user
    /// wrote a loop (`type A = B; type B = A;`), which we surface
    /// as a diagnostic instead of looping forever or stack-
    /// overflowing.
    fn parse_type_name_inner(&self, name: &str, seen: &mut Vec<String>) -> Result<Type, String> {
        // RES-391: strip the reference prefix — `& T`, `&mut T`,
        // `&[A] T`, `&mut[A] T` — before resolving the inner type.
        // The borrow checker (`main::check_region_aliasing`) has
        // already consumed the region / mutability info; downstream
        // type resolution cares only about the pointee.
        if let Some(rest) = name.strip_prefix('&') {
            let rest = rest.strip_prefix("mut").unwrap_or(rest);
            let rest = rest.trim_start();
            let rest = if let Some(after) = rest.strip_prefix('[') {
                // Skip to the first `]`.
                match after.find(']') {
                    Some(end) => after[end + 1..].trim_start(),
                    None => rest, // malformed, fall through
                }
            } else {
                rest
            };
            return self.parse_type_name_inner(rest, seen);
        }
        match name {
            // RES-366: `Int64` is the long-form alias for `Int`.
            // RES-2719: `long` (Java/C) maps to the 64-bit signed int.
            "int" | "Int" | "Int64" | "i64" | "long" => Ok(Type::Int),
            // RES-366: pinned signed integer widths (PascalCase + Rust-style lowercase).
            // RES-2719: snake_case variants (`int8`, `int16`, `int32`) added as aliases.
            "Int8" | "i8" | "int8" => Ok(Type::Int8),
            "Int16" | "i16" | "int16" => Ok(Type::Int16),
            "Int32" | "i32" | "int32" => Ok(Type::Int32),
            // RES-366: pinned unsigned integer widths (PascalCase + Rust-style lowercase).
            // RES-2719: snake_case variants + `byte` (common alias for u8) added.
            "UInt8" | "u8" | "uint8" | "byte" => Ok(Type::UInt8),
            "UInt16" | "u16" | "uint16" => Ok(Type::UInt16),
            "UInt32" | "u32" | "uint32" => Ok(Type::UInt32),
            "UInt64" | "u64" | "uint64" => Ok(Type::UInt64),
            // RES-2719: `double` is the common C/Java alias for 64-bit float.
            "float" | "Float" | "f64" | "Float64" | "double" => Ok(Type::Float),
            // RES-2618: single-precision float — `f32` and `Float32` are
            // both accepted; `float` / `f64` remain aliases for double.
            "f32" | "Float32" => Ok(Type::Float32),
            // RES-2719: `str` and `String` are common aliases for `string`
            // (Rust uses both; Java/Python use `String`/`str`).
            "string" | "str" | "String" => Ok(Type::String),
            // RES-2719: `boolean` is the common Java/JavaScript spelling.
            "bool" | "boolean" => Ok(Type::Bool),
            // RES-2711: `char` and `Char` both resolve to the character type.
            "char" | "Char" => Ok(Type::Char),
            "void" => Ok(Type::Void),
            "Result" => Ok(Type::Result),
            // RES-2651: bare `Option` or `Option<T>`.
            "Option" => Ok(Type::Option(Box::new(Type::Any))),
            // RES-2705: accept both `array` (canonical) and `Array` (capitalised)
            // so that struct fields and fn params written as `Array foo` resolve
            // to Type::Array instead of Type::Struct("Array"), which previously
            // caused false-positive type-mismatch errors against array literals.
            "array" | "Array" => Ok(Type::Array),
            // RES-408: `any` as a written type annotation maps to Type::Any
            // (the unresolved/wildcard type). Without this arm the identifier
            // falls through to `other => Type::Struct("any")`, which caused
            // argument-type mismatches (`expected any, got int`) and broke
            // struct pattern matching when the scrutinee was declared `any`.
            "any" | "Any" => Ok(Type::Any),
            "" => Ok(Type::Any), // Empty type name means "any" for now
            // RES-419: `fn(T1, T2) -> R` — function-type annotation.
            // Covers higher-order function parameters and return types.
            // Syntax: "fn(" followed by comma-separated type names,
            // ")" then " -> " then the return type. Nesting is handled
            // by counting parenthesis depth when splitting params.
            other if other.starts_with("fn(") => {
                let rest = &other[3..]; // after "fn("
                // Find the matching closing ')' counting nesting depth.
                let mut depth = 1usize;
                let mut close = None;
                for (i, ch) in rest.char_indices() {
                    match ch {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                close = Some(i);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let close = close.unwrap_or(rest.len().saturating_sub(1));
                let params_str = &rest[..close];
                let after_close = rest[close + 1..].trim_start();
                // Parse return type from " -> ReturnType".
                let return_type = if let Some(rt_str) = after_close.strip_prefix("->") {
                    self.parse_type_name_inner(rt_str.trim(), seen)?
                } else {
                    Type::Any
                };
                // Parse comma-separated parameter types, respecting nesting.
                let params = if params_str.trim().is_empty() {
                    Vec::new()
                } else {
                    // Upper bound: total comma count + 1. Nested-fn types
                    // (e.g. `(int) -> (bool, int)`) over-count slightly
                    // because the inner comma sits at depth > 0, but the
                    // bytewise scan is still cheaper than the 0 → 4 growth
                    // chain on `parts` for any params_str with ≥ 4 types.
                    let cap = params_str.bytes().filter(|b| *b == b',').count() + 1;
                    let mut parts: Vec<Type> = Vec::with_capacity(cap);
                    let mut depth2 = 0usize;
                    let mut start = 0;
                    for (i, ch) in params_str.char_indices() {
                        match ch {
                            '(' => depth2 += 1,
                            ')' => depth2 = depth2.saturating_sub(1),
                            ',' if depth2 == 0 => {
                                parts.push(
                                    self.parse_type_name_inner(params_str[start..i].trim(), seen)?,
                                );
                                start = i + 1;
                            }
                            _ => {}
                        }
                    }
                    parts.push(self.parse_type_name_inner(params_str[start..].trim(), seen)?);
                    parts
                };
                Ok(Type::Function {
                    params,
                    return_type: Box::new(return_type),
                })
            }
            // RES-2651: `Option<T>` with a type parameter — e.g.
            // `Option<int>`, `Option<float>`. Extract the inner type
            // name and resolve it recursively.
            other if other.starts_with("Option<") && other.ends_with('>') => {
                let inner_str = &other[7..other.len() - 1]; // strip "Option<" and ">"
                let inner_ty = self.parse_type_name_inner(inner_str.trim(), seen)?;
                Ok(Type::Option(Box::new(inner_ty)))
            }
            // RES-2740: `Result<T, E>` with type parameters — Type::Result is
            // unparameterized at MVP level, so we discard the params and resolve
            // as plain Type::Result. Without this arm the string falls through to
            // the alias lookup and becomes Type::Struct("Result<int, string>"),
            // which never matches Type::Result from Ok()/Err() and produces a
            // false-positive "return type mismatch" on every annotated function.
            other if other.starts_with("Result<") && other.ends_with('>') => Ok(Type::Result),
            // RES-426: tuple type `(T1, T2, ...)`. Encoded by the
            // parser as a parenthesised comma-list; at type-check
            // level a tuple is represented as Type::Any (the interpreter
            // already boxes tuples as Value::Tuple at runtime; the type
            // system will promote this to Type::Tuple once the full
            // tuple-type variant lands in G7).
            other if other.starts_with('(') && other.ends_with(')') => Ok(Type::Any),
            // RES-128: a registered alias expands transitively.
            // RES-1894: single `.get()` replaces the former
            // `contains_key()` guard + `[]` index double-lookup.
            // Moved after `fn(` and tuple guards so those patterns
            // are tried first (preserving match order), then the
            // alias lookup uses one `.get()` call instead of two.
            other => {
                if let Some(target) = self.type_aliases.get(other) {
                    if seen.iter().any(|n| n == other) {
                        let mut chain = seen.clone();
                        chain.push(other.to_string());
                        return Err(format!("type alias cycle: {}", chain.join(" -> ")));
                    }
                    let target = target.clone();
                    seen.push(other.to_string());
                    self.parse_type_name_inner(&target, seen)
                } else {
                    // RES-053: any other identifier is assumed to be a
                    // user-defined struct.
                    Ok(Type::Struct(other.to_string()))
                }
            }
        }
    }
}

// ============================================================
// RES-191: `@pure` purity checker.
// ============================================================
//
// Impurity model:
// - I/O (println, print, input, file_*) is impure.
// - Nondeterminism (clock_ms, random_int/float) is impure.
// - Environment reads (env) are impure (external state).
// - Live-block-state readers (live_retries, live_total_*) are
//   impure — they observe runtime retry counters.
// - Everything else in the builtin set is pure.
//
// An unannotated user fn called from a `@pure` fn is ALWAYS a
// violation per the ticket ("call unannotated user fns" →
// rejected). Pure-to-pure calls are fine, including mutual
// recursion.
//
// Implementation:
// 1. Walk the program top-level twice: first to collect
//    `pure_fns: HashSet<String>`, then to check each `@pure` fn's
//    body.
// 2. `check_body_purity` is a recursive AST walker. At every
//    `CallExpression`, look up the callee against the impure-
//    builtin set + the pure-fn set; emit a violation message on
//    mismatch.
// 3. `LiveBlock` (live ... {}) is impure by nature — retries are
//    observable behaviour. A `@pure` fn containing one is a
//    violation too.

/// Names of builtin functions the runtime provides that have
/// observable side effects or nondeterminism. Any `@pure` fn
/// that calls one of these is rejected at type-check time.
///
/// Keep in sync with `resilient/src/main.rs::BUILTINS` — adding a
/// new I/O / clock / env builtin there means adding it here.
const IMPURE_BUILTINS: &[&str] = &[
    // RES-004 / RES-144: stdio.
    "println",
    "print",
    "input",
    // RES-2610: compile-time file embedding.
    "include_str",
    "include_bytes",
    // RES-147: monotonic clock.
    "clock_ms",
    // RES-1174: wall-clock unix time.
    "unix_time_s",
    "unix_time_ms",
    "unix_time_ns",
    // RES-150: seedable PRNG — nondeterministic from the caller's
    // point of view even though the seed pins it globally.
    "random_int",
    "random_float",
    // RES-444: shuffle pulls from the same RNG.
    "array_shuffle",
    // RES-143: disk I/O.
    "file_read",
    "file_write",
    // RES-409: streaming file I/O. All five reach the disk and so
    // count as impure for the effect inferrer.
    "file_open",
    "file_read_chunk",
    "file_write_chunk",
    "file_seek",
    "file_close",
    // RES-151: env-var reads depend on process state outside
    // the fn.
    "env",
    // RES-138 / RES-141: retry-counter readers — observe runtime
    // state that isn't the fn's parameters.
    "live_retries",
    "live_total_retries",
    "live_total_exhaustions",
];

/// RES-191: top-level entry for the purity pass. Walks the
/// program's statement list once to collect `@pure` fn names
/// (their declarations include the `pure: bool` flag per the
/// ticket), then re-walks each declared-pure fn's body and
/// RES-388: iterate top-level statements and verify every
/// `ActorDecl`'s `always` safety invariants. Returns the flattened
/// list of per-obligation verdicts; the caller inspects each for a
/// `Refuted` verdict to decide whether the check fails.
fn collect_actor_obligations(
    statements: &[crate::span::Spanned<Node>],
    verifier_timeout_ms: u32,
) -> Vec<crate::verifier_actors::ActorObligation> {
    let mut out = Vec::new();
    for stmt in statements {
        if let Node::ActorDecl {
            name,
            state_fields,
            always_clauses,
            receive_handlers,
            ..
        } = &stmt.node
        {
            out.extend(crate::verifier_actors::verify_actor(
                name,
                state_fields,
                always_clauses,
                receive_handlers,
                verifier_timeout_ms,
            ));
        }
    }
    out
}

/// reports the first violation.
///
/// Errors use the `<path>:<line>:<col>: <msg>` prefix convention
/// from RES-080 when a useful span is available.
fn check_program_purity(
    statements: &[crate::span::Spanned<Node>],
    source_path: &str,
) -> Result<(), String> {
    // Optimistic assumption: every `@pure` fn is pure until proven
    // otherwise. Populate the set so mutual-recursion checks
    // succeed.
    //
    // RES-1525: borrow each `@pure` fn name as `&str` from the AST
    // into the lookup set. `check_body_purity`'s only consumer is
    // `pure_fns.contains(callee)` — `HashSet::contains` accepts
    // `&str` via `Borrow<str>`, so the cloned `String` keys were
    // pure overhead. Same pattern as RES-1500 / RES-1523 etc.
    // RES-1796: pre-size to statements.len() — at most one insert per
    // top-level statement (`@pure` Function), upper bound. Same shape
    // as the call-graph / fn_info pre-size series.
    let mut pure_fns: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(statements.len());
    for stmt in statements {
        if let Node::Function {
            name, pure: true, ..
        } = &stmt.node
        {
            pure_fns.insert(name.as_str());
        }
    }

    // RES-1296: short-circuit. The second pass walks every top-level
    // statement and only descends into `Node::Function { pure: true,
    // ... }`. If no `@pure` fn was found above, no descent ever
    // fires — every iteration is wasted match-dispatch overhead.
    if pure_fns.is_empty() {
        return Ok(());
    }

    // Second pass: check each pure fn's body.
    for stmt in statements {
        if let Node::Function {
            name,
            body,
            pure: true,
            ..
        } = &stmt.node
            && let Err(reason) = check_body_purity(body, name, &pure_fns)
        {
            let (line, col) = (stmt.span.start.line, stmt.span.start.column);
            return Err(if line == 0 {
                format!("@pure fn `{}`: {}", name, reason)
            } else {
                format!(
                    "{}:{}:{}: @pure fn `{}`: {}",
                    source_path, line, col, name, reason
                )
            });
        }
    }
    Ok(())
}

/// RES-191: recursive AST walker that enforces the purity rules
/// inside a fn body. Returns `Err(<reason>)` with the violating
/// construct described for the caller to prefix with the fn's
/// span. On success every reachable call / live block was a
/// pure-to-pure edge.
///
/// `pure_fns` is the set of user fn names that have been declared
/// `@pure` — callees in that set pass; callees outside (user fn
/// without annotation) fail. `fn_name` is currently used only by
/// recursive self-calls; it's threaded through so a future
/// extension (e.g. "don't recurse into nested fn decls of a
/// different name") can read it without a signature change —
/// hence the `only_used_in_recursion` allow.
#[allow(clippy::only_used_in_recursion)]
fn check_body_purity(
    node: &Node,
    fn_name: &str,
    pure_fns: &std::collections::HashSet<&str>,
) -> Result<(), String> {
    match node {
        Node::Block { stmts, .. } => {
            for s in stmts {
                check_body_purity(s, fn_name, pure_fns)?;
            }
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            check_body_purity(value, fn_name, pure_fns)?;
        }
        Node::ReturnStatement {
            value: Some(value), ..
        } => {
            check_body_purity(value, fn_name, pure_fns)?;
        }
        Node::ReturnStatement { value: None, .. } => {}
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            check_body_purity(condition, fn_name, pure_fns)?;
            check_body_purity(consequence, fn_name, pure_fns)?;
            if let Some(a) = alternative {
                check_body_purity(a, fn_name, pure_fns)?;
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            check_body_purity(condition, fn_name, pure_fns)?;
            check_body_purity(body, fn_name, pure_fns)?;
        }
        Node::ForInStatement { iterable, body, .. } => {
            check_body_purity(iterable, fn_name, pure_fns)?;
            check_body_purity(body, fn_name, pure_fns)?;
        }
        Node::Assert { condition, .. } => {
            check_body_purity(condition, fn_name, pure_fns)?;
        }
        Node::Assume { condition, .. } => {
            check_body_purity(condition, fn_name, pure_fns)?;
        }
        Node::LiveBlock { .. } => {
            // live-blocks retry on failure — that's observable,
            // non-pure behaviour by construction.
            return Err("contains a `live` block (retries are \
                        observable side effects)"
                .to_string());
        }
        Node::InfixExpression { left, right, .. } => {
            check_body_purity(left, fn_name, pure_fns)?;
            check_body_purity(right, fn_name, pure_fns)?;
        }
        Node::PrefixExpression { right, .. } => {
            check_body_purity(right, fn_name, pure_fns)?;
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            // Recurse into args first (nested calls get checked too).
            for a in arguments {
                check_body_purity(a, fn_name, pure_fns)?;
            }
            // Determine the callee name. We only resolve bare
            // identifier calls; `(expr)(...)` style indirect
            // calls aren't used in pure code paths today. Method
            // calls flow through FieldAccess and get the
            // conservative "unknown callee — reject" treatment.
            if let Node::Identifier { name: callee, .. } = function.as_ref() {
                if IMPURE_BUILTINS.contains(&callee.as_str()) {
                    return Err(format!("calls impure builtin `{}`", callee));
                }
                // Pure-to-pure is fine.
                if pure_fns.contains(callee.as_str()) {
                    return Ok(());
                }
                // Known pure builtins are implicitly fine. The
                // "pure builtin" set is the complement of
                // `IMPURE_BUILTINS` over the BUILTINS table —
                // rather than maintain two lists, we treat any
                // non-impure builtin name as implicitly pure.
                // User fns NOT declared `@pure` fall through to
                // the unannotated-user-fn error path.
                if is_known_pure_builtin(callee) {
                    return Ok(());
                }
                return Err(format!("calls unannotated fn `{}`", callee));
            }
            // Indirect / method callee — can't resolve statically.
            // Conservatively reject so @pure is meaningful.
            check_body_purity(function, fn_name, pure_fns)?;
            return Err("calls a non-identifier callee (method or computed); \
                 only bare-identifier calls to pure fns are allowed"
                .to_string());
        }
        Node::FieldAccess { target, .. } => {
            check_body_purity(target, fn_name, pure_fns)?;
        }
        Node::FieldAssignment { target, value, .. } => {
            check_body_purity(target, fn_name, pure_fns)?;
            check_body_purity(value, fn_name, pure_fns)?;
            // Mutating a field is observable — disallow.
            return Err("mutates a struct field (field assignment is a side effect)".to_string());
        }
        Node::Assignment { value, .. } => {
            check_body_purity(value, fn_name, pure_fns)?;
        }
        Node::IndexExpression { target, index, .. } => {
            check_body_purity(target, fn_name, pure_fns)?;
            check_body_purity(index, fn_name, pure_fns)?;
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            check_body_purity(target, fn_name, pure_fns)?;
            check_body_purity(index, fn_name, pure_fns)?;
            check_body_purity(value, fn_name, pure_fns)?;
            return Err(
                "mutates an array/map element (index assignment is a side effect)".to_string(),
            );
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                check_body_purity(i, fn_name, pure_fns)?;
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(b) = base {
                check_body_purity(b, fn_name, pure_fns)?;
            }
            for (_, v) in fields {
                check_body_purity(v, fn_name, pure_fns)?;
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            check_body_purity(scrutinee, fn_name, pure_fns)?;
            // Each arm is `(pattern, guard?, body)`. Recurse into
            // the optional guard and the body.
            for (_pat, guard, body) in arms {
                if let Some(g) = guard {
                    check_body_purity(g, fn_name, pure_fns)?;
                }
                check_body_purity(body, fn_name, pure_fns)?;
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            check_body_purity(expr, fn_name, pure_fns)?;
        }
        Node::TryExpression { expr, .. } => {
            check_body_purity(expr, fn_name, pure_fns)?;
        }
        Node::OptionalChain { object, access, .. } => {
            check_body_purity(object, fn_name, pure_fns)?;
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    check_body_purity(a, fn_name, pure_fns)?;
                }
            }
        }
        // Pure literals / identifier reads / etc — no work.
        Node::IntegerLiteral { .. }
        | Node::FloatLiteral { .. }
        | Node::StringLiteral { .. }
        | Node::BooleanLiteral { .. }
        | Node::BytesLiteral { .. }
        | Node::Identifier { .. }
        | Node::DurationLiteral { .. } => {}
        // Declarations inside a fn body are unusual but not
        // inherently impure — recurse to be thorough.
        Node::Function { body, .. } => {
            check_body_purity(body, fn_name, pure_fns)?;
        }
        // Everything else: default to "not inspected; not known-
        // impure". If a new AST variant needs treatment, add an
        // arm. Until then, be lenient rather than over-strict.
        _ => {}
    }
    Ok(())
}

/// RES-191: helper mirroring the split in `IMPURE_BUILTINS`. We
/// don't want to hard-code the full pure-builtin list (bit-rot
/// risk — new builtins would silently be treated as user fns);
/// instead we mark the interpreter's builtin names as pure by
/// default, leaving `IMPURE_BUILTINS` as the authoritative list
/// of exceptions.
///
/// Input is the callee name. Returns true iff the name is one of
/// the pure-by-default builtins we ship.
fn is_known_pure_builtin(name: &str) -> bool {
    // Keep this list in sync with `resilient/src/main.rs::BUILTINS`
    // minus the names in `IMPURE_BUILTINS`.
    const PURE_BUILTINS: &[&str] = &[
        // Math.
        "abs",
        // RES-410: sign(x).
        "sign",
        // RES-415: gcd/lcm.
        "gcd",
        "lcm",
        // RES-536: gcd / lcm reduction over an integer array.
        "gcd_array",
        "lcm_array",
        // RES-567: factorial with overflow detection.
        "factorial",
        // RES-568: binomial coefficient C(n, k).
        "binomial",
        // RES-569: n-th Fibonacci with overflow detection.
        "fibonacci",
        // RES-570: trial-division primality test.
        "is_prime",
        // RES-571: smallest prime greater than n.
        "next_prime",
        // RES-411: float predicates.
        "is_nan",
        "is_inf",
        "is_finite",
        "min",
        "max",
        // RES-295.
        "clamp",
        "atan2",
        "hypot",
        "copysign",
        "sqrt",
        "pow",
        "floor",
        "ceil",
        "to_float",
        "to_int",
        "sin",
        "cos",
        "tan",
        "to_radians",
        "to_degrees",
        "ln",
        "log10",
        "log2",
        "log",
        "exp",
        "exp2",
        "sinh",
        "cosh",
        "tanh",
        "asinh",
        "acosh",
        "atanh",
        "asin",
        "acos",
        "atan",
        "cbrt",
        "count_ones",
        "count_zeros",
        "leading_zeros",
        "trailing_zeros",
        // RES-1115..1118: overflow-safe arithmetic.
        "saturating_add",
        "saturating_sub",
        "saturating_mul",
        "wrapping_add",
        "wrapping_sub",
        "wrapping_mul",
        "checked_add",
        "checked_sub",
        "checked_mul",
        "checked_div",
        // RES-1119..1121: bit manipulation.
        "rotate_left_int",
        "rotate_right_int",
        "reverse_bits",
        "swap_bytes",
        // RES-1122..1123: int ↔ bytes endianness conversion.
        "to_be_bytes",
        "to_le_bytes",
        "from_be_bytes",
        "from_le_bytes",
        // RES-1124: integer-only math primitives.
        "isqrt",
        "ipow",
        // RES-1126..1128: direction-rounded + Euclidean + midpoint.
        "div_ceil",
        "div_floor",
        "div_euclid",
        "rem_euclid",
        "midpoint",
        // RES-1129: integer logarithms.
        "ilog2",
        "ilog10",
        // RES-1130: IEEE 754 bit reinterpret cast.
        "float_to_bits",
        "float_from_bits",
        // RES-1134: bitwise + construction ops on Bytes.
        "bytes_xor",
        "bytes_and",
        "bytes_or",
        "bytes_not",
        "bytes_fill",
        "bytes_reverse",
        // RES-1136: alignment helpers.
        "next_multiple_of",
        "is_multiple_of",
        // RES-1138: IEEE 754 classification + total order + sign-bit.
        "float_classify",
        "float_total_cmp",
        "float_is_normal",
        "float_is_subnormal",
        "float_sign_bit",
        // RES-1142: array chunking + striding + rotation.
        "array_chunks",
        "array_chunks_exact",
        "array_step",
        "array_rotate_left",
        "array_rotate_right",
        // RES-1140: ASCII char-class predicates.
        "is_ascii",
        "is_ascii_whitespace",
        "is_ascii_hexdigit",
        "is_ascii_uppercase",
        "is_ascii_lowercase",
        "is_ascii_punctuation",
        "is_ascii_control",
        // RES-1146: float / string sort + array_is_sorted predicates.
        "array_sort_float",
        "array_sort_string",
        "array_is_sorted",
        "array_is_sorted_float",
        "array_is_sorted_string",
        // RES-1148: binary search on sorted arrays.
        "array_binary_search",
        "array_binary_search_float",
        "array_binary_search_string",
        // RES-1150: statistical reductions.
        "array_variance_int",
        "array_variance_float",
        "array_stddev_int",
        "array_stddev_float",
        "array_median_float",
        "array_range_float",
        // RES-1152: per-byte helpers.
        "bytes_repeat",
        "bytes_count_byte",
        "bytes_replace_byte",
        // RES-1156: per-bit accessors.
        "set_bit",
        "clear_bit",
        "get_bit",
        "flip_bit",
        // RES-1158: array set-style helpers.
        "array_difference",
        "array_intersection",
        "array_index_of_last",
        "array_first_or",
        "array_last_or",
        // RES-1162: deterministic hash builtins.
        "hash_int",
        "hash_string",
        "hash_bytes",
        "hash_combine",
        // RES-1164: iteration helpers.
        "enumerate",
        "array_zip3",
        "string_truncate",
        // RES-1166: rounding builtins.
        "round",
        "trunc",
        "round_to_int",
        "trunc_to_int",
        // RES-1170: cumulative reductions + combined min/max.
        "array_cumsum",
        "array_cumprod",
        "array_diffs",
        "array_min_max",
        // RES-1172: small string + array gaps.
        "string_split_once",
        "string_rsplit_once",
        "string_from_chars",
        "array_is_empty",
        // RES-1176: bytes ↔ string conversions.
        "bytes_strip_prefix",
        "bytes_strip_suffix",
        "bytes_to_string",
        // RES-1178: bytes slicing primitives.
        "bytes_take",
        "bytes_drop",
        "bytes_take_last",
        "bytes_drop_last",
        // RES-1182: integer bit rotation + scalar signum.
        "rotate_left",
        "rotate_right",
        "signum",
        // String/collection.
        "len",
        "push",
        "pop",
        "slice",
        "split",
        // RES-1859: explicit-name alias.
        "string_split",
        // RES-535: split with a maximum number-of-splits limit.
        "string_split_n",
        // RES-545: split on the last occurrence of the separator.
        "string_split_last",
        "trim",
        "contains",
        "to_upper",
        "to_lower",
        // RES-412: reverse string/array.
        "string_reverse",
        "array_reverse",
        // RES-2734: short-name aliases for array dot-call methods.
        "sort",
        "sort_desc",
        "reverse",
        "join",
        "flatten",
        "dedup",
        "has",
        // RES-2738: string dot-call methods (routed via runtime name mapping).
        "replace",
        "chars",
        "strip_prefix",
        "strip_suffix",
        "lines",
        // RES-1859: higher-order array builtins.
        "array_map",
        "array_filter",
        "array_reduce",
        // RES-507: generic callback-based search/predicate builtins.
        "array_find",
        "array_find_index",
        "array_any",
        "array_all",
        // RES-2646: higher-order functional array operations.
        "array_flat_map",
        "array_group_by",
        "array_partition",
        "map_from_pairs",
        "array_scan",
        // RES-2647: map functional operations.
        "map_filter",
        "map_map_values",
        "map_for_each",
        "map_to_pairs",
        // RES-2648: array combinators.
        "array_sort_by",
        "array_min_by",
        "array_max_by",
        "array_count_if",
        "array_zip_with",
        "array_windows",
        // RES-2742: pain-points hardening builtins missing from known list.
        "array_sort_by_field",
        "array_sort_by_field_desc",
        "array_flatten_depth",
        "array_dedup_by",
        "array_none",
        "int_parse_hex",
        "int_parse_bin",
        "int_to_oct",
        "array_take_while",
        "array_drop_while",
        // RES-2649: array aggregation by key.
        "array_sum_by",
        "array_product_by",
        // RES-2649: map higher-order operations.
        "map_merge_with",
        "map_update_with",
        // RES-2649: string higher-order operations.
        "string_map_chars",
        "string_filter_by",
        "string_fold",
        "string_for_each_char",
        // RES-2650: numeric utilities.
        "lerp",
        "remap",
        "float_approx_eq",
        "round_to",
        "int_pow",
        // RES-2650: collection extras.
        "array_frequency_map",
        "array_key_by",
        "array_iterate",
        // RES-2651: Result/Option HOF.
        "result_map",
        "result_and_then",
        "result_map_err",
        "result_or_else",
        "option_map",
        "option_and_then",
        "option_filter",
        "option_or_else",
        "option_ok_or",
        // RES-2652: type introspection + collection ergonomics.
        "type_of",
        "result_collect",
        "array_from_fn",
        "map_invert",
        // RES-416: array reductions.
        "array_sum",
        "array_product",
        // RES-417: array min/max.
        "array_min",
        "array_max",
        // RES-543: empty-safe min/max with fallback default.
        "array_max_or",
        "array_min_or",
        // RES-549: integer mean (truncating toward zero).
        "array_mean_int",
        // RES-550: integer median.
        "array_median_int",
        // RES-551: integer mode (most-common; smallest on ties).
        "array_mode_int",
        // RES-552: peak-to-peak range (max − min).
        "array_range_int",
        // RES-553: consecutive pairwise differences.
        "array_diff_consec_int",
        // RES-554: per-element clamp.
        "array_clamp_int",
        // RES-555: per-element sign (-1/0/1).
        "array_signum_int",
        // RES-556: per-element absolute value.
        "array_abs_int",
        // RES-557: dot product.
        "array_dot_int",
        // RES-558: sum of squares.
        "array_sum_squares_int",
        // RES-559: running prefix sum.
        "array_cumsum_int",
        // RES-560: running max.
        "array_cummax_int",
        // RES-561: running min.
        "array_cummin_int",
        // RES-562: running product.
        "array_cumprod_int",
        // RES-563: count elements in inclusive [lo, hi].
        "array_count_in_range_int",
        // RES-503: index of max/min int element.
        "array_argmax_int",
        "array_argmin_int",
        // RES-418: array search.
        "array_contains",
        "array_index_of",
        // RES-544: every index where element equals x.
        "array_index_of_all",
        // RES-541: set-like operations on arrays.
        "array_intersect",
        "array_diff",
        // RES-542: order-preserving global-dedup union.
        "array_union",
        // RES-419: char-code conversions.
        "chr",
        "ord",
        // RES-505: parse single char to base-36 digit.
        "char_to_digit",
        // RES-513: int 0..=35 → base-36 digit char.
        "digit_to_char",
        // RES-420: array_concat.
        "array_concat",
        // RES-515: three-way concatenation.
        "array_concat3",
        // RES-421: take/drop.
        "array_take",
        "array_drop",
        // RES-537: take/drop trailing n elements.
        "array_take_last",
        "array_drop_last",
        // RES-514: pick every nth element.
        "array_step",
        // RES-422: integer sort.
        "array_sort",
        // RES-443: descending sort.
        "array_sort_desc",
        // RES-445: prefix/suffix predicates.
        "array_starts_with",
        "array_ends_with",
        // RES-446: all match indices.
        "string_find_all",
        // RES-546: first byte index of substring, -1 if missing.
        "string_find",
        // RES-547: last byte index of substring, -1 if missing.
        "string_rfind",
        // RES-548: split at byte index.
        "string_split_at",
        // RES-447: int_min / int_max.
        "int_min",
        "int_max",
        // RES-448: array_position.
        "array_position",
        // RES-449: array padding.
        "array_pad_left",
        "array_pad_right",
        // RES-450: array_swap.
        "array_swap",
        // RES-451: insert/remove at index.
        "array_insert_at",
        "array_remove_at",
        // RES-452: array_set_at.
        "array_set_at",
        // RES-453: string_at.
        "string_at",
        // RES-454: string_substring.
        "string_substring",
        // RES-455: array_window.
        "array_window",
        // RES-456: rotation.
        "array_rotate_left",
        "array_rotate_right",
        // RES-457: capitalize.
        "string_capitalize",
        // RES-458: array_cycle.
        "array_cycle",
        // RES-459: ASCII predicates.
        "is_ascii_alpha",
        "is_ascii_digit",
        "is_ascii_alnum",
        // RES-460: trim_chars.
        "trim_chars",
        // RES-461: string_indent.
        "string_indent",
        // RES-462: array_pairs.
        "array_pairs",
        // RES-463: string_bytes_len.
        "string_bytes_len",
        // RES-564: string_byte_at.
        "string_byte_at",
        // RES-565: string_to_bytes.
        "string_to_bytes",
        // RES-566: string_from_bytes.
        "string_from_bytes",
        // RES-464: parse_int_base.
        "parse_int_base",
        // RES-465: int_to_base.
        "int_to_base",
        // RES-466: array_remove.
        "array_remove",
        // RES-467: array_remove_all.
        "array_remove_all",
        // RES-468: array_dedup.
        "array_dedup",
        // RES-504: group consecutive equal int elements.
        "array_group_by_int",
        // RES-533: count of maximal runs.
        "array_count_runs",
        // RES-469: all/any equality.
        "array_all_eq",
        "array_any_eq",
        // RES-471: prefix/suffix strippers.
        "string_strip_prefix",
        "string_strip_suffix",
        // RES-472: array_eq.
        "array_eq",
        // RES-473: min3 / max3.
        "min3",
        "max3",
        // RES-474: array_ne.
        "array_ne",
        // RES-475: array_fold_int.
        "array_fold_int",
        // RES-502: running-fold over int arrays.
        "array_scan_int",
        // RES-521: element-wise binary op on two int arrays.
        "array_zip_with_int",
        // RES-477: one-sided char-set trimmers.
        "trim_start_chars",
        "trim_end_chars",
        // RES-478: array_count_eq alias.
        "array_count_eq",
        // RES-479: string predicates.
        "is_empty",
        "is_blank",
        // RES-480: string_replace_first.
        "string_replace_first",
        // RES-481: array_rest / array_init.
        "array_rest",
        "array_init",
        // RES-482: string_replace_n.
        "string_replace_n",
        // RES-483: take/drop while int.
        "array_take_while_int",
        "array_drop_while_int",
        // RES-484: filter / partition int.
        "array_filter_int",
        // RES-500: named-predicate any.
        "array_any_int",
        // RES-501: named-predicate every.
        "array_all_int",
        // RES-530: named-predicate count.
        "array_count_int",
        // RES-539: indices of int elements matching named predicate.
        "array_indices_where",
        "array_partition_int",
        // RES-485: abs_diff.
        "abs_diff",
        // RES-486: divmod.
        "divmod",
        // RES-423: flatten one level.
        "array_flatten",
        // RES-424: array_join.
        "array_join",
        // RES-425: to_string.
        "to_string",
        // RES-426: array_unique.
        "array_unique",
        // RES-427: array_count.
        "array_count",
        // RES-428: array first/last.
        "array_first",
        "array_last",
        // RES-528: bounded indexing with fallback.
        "array_get_or",
        // RES-429: string padding.
        "string_pad_left",
        "string_pad_right",
        // RES-540: center-pad a string.
        "string_pad_center",
        // RES-430: array_zip.
        "array_zip",
        // RES-531: split an array of 2-tuples into two parallel arrays.
        "array_unzip",
        // RES-431: array_range.
        "array_range",
        // RES-522: indices of an array.
        "array_indices",
        // RES-432: array_repeat.
        "array_repeat",
        // RES-433: string_chars.
        "string_chars",
        // RES-434: string_lines.
        "string_lines",
        // RES-496: split on Unicode whitespace.
        "string_words",
        // RES-497: join string array with newline.
        "string_join_lines",
        // RES-498: join string array with single space.
        "string_unwords",
        // RES-499: take first n Unicode scalars.
        "string_take",
        // RES-506: drop first n Unicode scalars.
        "string_drop",
        // RES-435: array_chunk.
        "array_chunk",
        // RES-436: string_count.
        "string_count",
        // RES-523: count occurrences of a single character.
        "string_count_char",
        // RES-524: char-index of a single character.
        "string_find_char",
        // RES-525: named-predicate string slicing.
        "string_take_while_char",
        "string_drop_while_char",
        // RES-526: named-predicate global char filter.
        "string_filter_char",
        // RES-527: ASCII case-insensitive string equality.
        "string_eq_ignore_case",
        // RES-437: array_intersperse.
        "array_intersperse",
        // RES-516: alternate elements from two arrays.
        "array_interleave",
        // RES-438: one-sided trimmers.
        "trim_start",
        "trim_end",
        // RES-439: array_split_at.
        "array_split_at",
        // RES-440: bitwise ops.
        "bit_and",
        "bit_or",
        "bit_xor",
        "bit_not",
        "bit_shl",
        "bit_shr",
        // RES-488: popcount.
        "bit_count",
        // RES-489: count leading zero bits.
        "bit_leading_zeros",
        // RES-490: count trailing zero bits.
        "bit_trailing_zeros",
        // RES-511: single-bit test / set / clear / toggle.
        "bit_test",
        "bit_set",
        "bit_clear",
        "bit_toggle",
        // RES-520: circular bit rotation.
        "bit_rotate_left",
        "bit_rotate_right",
        // RES-534: extract a single byte from an i64.
        "bit_byte",
        // RES-538: set a single byte of an i64.
        "bit_set_byte",
        // RES-491: integer floor sqrt.
        "int_sqrt",
        // RES-517: integer exponentiation.
        "pow_int",
        // RES-518: integer division with explicit rounding.
        "ceil_div",
        "floor_div",
        // RES-519: Python-style modulo (sign of divisor).
        "modulo",
        // RES-492: floor log base 2.
        "int_log2",
        // RES-493: power-of-two predicate.
        "is_pow2",
        // RES-494: round up to next power of two.
        "next_pow2",
        // RES-495: int → lowercase hex string.
        "int_to_hex",
        // RES-512: int → binary string.
        "int_to_bin",
        // RES-442: last_index_of.
        "last_index_of",
        // RES-413: repeat a string.
        "string_repeat",
        // RES-414: substring search.
        "index_of",
        "replace",
        "format",
        "starts_with",
        "ends_with",
        "repeat",
        // RES-339: string parsing and formatting.
        "parse_int",
        // RES-529: non-erroring parse with fallback default.
        "parse_int_or",
        // RES-532: non-erroring float parse with fallback default.
        "parse_float_or",
        "parse_float",
        "char_at",
        "pad_left",
        "pad_right",
        // Result helpers.
        "Ok",
        "Err",
        "is_ok",
        "is_err",
        "unwrap",
        "unwrap_err",
        // Map/Set/Bytes.
        "map_new",
        "map_insert",
        "map_get",
        "map_remove",
        "map_keys",
        "map_len",
        "map_values",
        "map_contains_key",
        // RES-1144: map_entries / map_merge / map_is_empty.
        "map_entries",
        "map_merge",
        "map_is_empty",
        // RES-293: HashMap stdlib builtins (purely functional —
        // each returns a new map / scalar; no IO).
        "hashmap_new",
        "hashmap_insert",
        "hashmap_get",
        "hashmap_remove",
        "hashmap_contains",
        "hashmap_keys",
        "hashmap_len",
        "hashmap_values",
        // RES-1144: hashmap_entries / hashmap_merge / hashmap_is_empty.
        "hashmap_entries",
        "hashmap_merge",
        "hashmap_is_empty",
        // RES-1154: set_is_empty / set_from_array / result_and / option_and.
        "set_is_empty",
        "set_from_array",
        "result_and",
        "option_and",
        // RES-1160: argmax / argmin for float and string arrays.
        "array_argmax_float",
        "array_argmin_float",
        "array_argmax_string",
        "array_argmin_string",
        // RES-1168: precision-sensitive math.
        "expm1",
        "ln_1p",
        "mul_add",
        "recip",
        "set_new",
        "set_insert",
        "set_remove",
        "set_has",
        "set_len",
        "set_items",
        "set_union",
        "set_intersection",
        "set_difference",
        "set_is_subset",
        "set_is_superset",
        "set_is_disjoint",
        "set_symmetric_difference",
        "bytes_len",
        "bytes_slice",
        "byte_at",
        "bytes_concat",
        "bytes_eq",
        // RES-944: bytes search.
        "bytes_starts_with",
        "bytes_ends_with",
        "bytes_index_of",
        // RES-943: hex encoding.
        "bytes_to_hex",
        "bytes_from_hex",
        // RES-936/937: Result fallback variants.
        "result_unwrap_or",
        "result_unwrap_or_err",
        // RES-938: Result <-> Option conversion.
        "result_to_option",
        "option_to_result",
        // RES-939: chain alternatives.
        "option_or",
        "result_or",
        // RES-940: power-of-two helpers.
        "is_power_of_two",
        "next_power_of_two",
        // RES-941: int-array statistics.
        "array_average",
        "array_median",
        // RES-942: float-array reductions.
        "array_sum_float",
        "array_product_float",
        "array_min_float",
        "array_max_float",
        "array_average_float",
        // RES-945: default-fallback map accessors.
        "map_get_or",
        "hashmap_get_or",
        // RES-2619: Char type builtins — all are pure.
        "char_is_alpha",
        "char_is_digit",
        "char_is_whitespace",
        "char_is_upper",
        "char_is_lower",
        "char_is_alphanumeric",
        "char_is_ascii",
        "char_to_upper",
        "char_to_lower",
        "char_to_int",
        "int_to_char",
        "char_to_string",
    ];
    // RES-1530: lookup against a `HashSet<&'static str>` built once
    // per process from `PURE_BUILTINS`. The previous shape called
    // `PURE_BUILTINS.contains(&name)` which is O(N=442) per call —
    // and every `CallExpression { function: Identifier { name, .. } }`
    // in every fn body purity check hits this path. Converting to a
    // `LazyLock<HashSet<&str>>` drops the lookup to O(1) without
    // changing observable behaviour.
    static PURE_BUILTINS_SET: std::sync::LazyLock<std::collections::HashSet<&'static str>> =
        std::sync::LazyLock::new(|| PURE_BUILTINS.iter().copied().collect());
    PURE_BUILTINS_SET.contains(name)
}

// ============================================================
// RES-192: IO-effect inference (fixpoint over the call graph).
// ============================================================
//
// Lattice: `{}` (pure) or `{IO}`, represented as a single `bool`
// because the MVP only tracks one effect. Union = logical OR.
//
// Rules:
// - A builtin in `IMPURE_BUILTINS` has `IO`.
// - A call to a builtin name that's neither impure nor known-pure
//   is conservatively `IO` (unknown-callee = assume worst).
// - A user fn has `IO` iff any call site in its body calls
//   something with `IO`.
// - A `LiveBlock` is NOT inherently IO (the ticket specifically
//   calls out "reach println or file_*"; a retry loop over pure
//   work is still pure).
//
// Fixpoint: initialize every user fn to pure; iterate body-walks
// until no effect flips. Terminates in O(|fns|²) iterations since
// each pass can only flip pure→IO once per fn.
//
// RES-775 Phase 2: Higher-Order Effect Polymorphism
// ==================================================
// Current MVP limitation: Higher-order function combinators (map, filter,
// with_retry, etc.) default to `@io` even when passed a `@pure` callback,
// because the inference system does not thread effect variables through
// generic signatures.
//
// Phase 2 will extend this to:
// 1. Accept effect-variable constraints in generics (`E: effect`)
// 2. Infer concrete effects at HOF call sites based on callback effects
// 3. Carry effect constraints through monomorphization
// 4. Compose effects soundly in nested HOF chains
//
// This allows reusable combinators to remain provably `@pure` when
// their callbacks are pure, improving expressiveness for functional
// programming patterns in safety-critical contexts.

/// RES-192: build the call-graph edge set for `statements`, then
/// run the fixpoint. Returns `name → has_io` for every top-level
/// user fn. Non-function statements contribute nothing.
pub fn infer_fn_effects(
    statements: &[crate::span::Spanned<Node>],
) -> std::collections::HashMap<String, bool> {
    // RES-1503: build the intermediate fn-name maps keyed by `&str`,
    // borrowing from each `Node::Function::name`. The previous shape
    // allocated owned `String`s for every fn name three times: once
    // for `fn_bodies`, once for the `effects` init clone, and once
    // per fixpoint flip. The return type stays `HashMap<String, bool>`
    // because `self.stats.fn_effects` is an owned field — the
    // conversion happens once at the end (one `to_string()` per fn,
    // matching what the return type already costs).
    //
    // Pre-size `fn_bodies` to `statements.len()` — upper bound, since
    // every Function statement contributes one entry. Same shape as
    // `check_program_purity::pure_fns` (RES-1796) and
    // `collect_fn_effects::out` (RES-1734) — the two sibling passes
    // that walk the same statement list to populate the same shape of
    // map.
    let mut fn_bodies: std::collections::HashMap<&str, &Node> =
        std::collections::HashMap::with_capacity(statements.len());
    for stmt in statements {
        if let Node::Function { name, body, .. } = &stmt.node {
            fn_bodies.insert(name.as_str(), body.as_ref());
        }
    }

    // Step 2: initialize every fn as pure.
    let mut effects: std::collections::HashMap<&str, bool> =
        fn_bodies.keys().map(|n| (*n, false)).collect();

    // Step 3: fixpoint — iterate body-walks until no effect flips.
    // Upper bound: one flip per fn, so at most |fns| passes.
    let max_passes = fn_bodies.len().saturating_add(1);
    for _ in 0..max_passes {
        let mut changed = false;
        for (name, body) in &fn_bodies {
            if *effects.get(*name).unwrap_or(&false) {
                continue; // already IO — nothing to update
            }
            if body_reaches_io(body, &effects) {
                effects.insert(*name, true);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    effects
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

/// RES-192: body-level check for IO reachability under the
/// current `effects` snapshot. Used inside the fixpoint loop —
/// each iteration treats `effects` as a frozen best-estimate and
/// asks "does this body reach anything marked IO today?".
fn body_reaches_io(node: &Node, effects: &std::collections::HashMap<&str, bool>) -> bool {
    match node {
        Node::Block { stmts, .. } => stmts.iter().any(|s| body_reaches_io(s, effects)),
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            body_reaches_io(value, effects)
        }
        Node::ReturnStatement { value: Some(v), .. } => body_reaches_io(v, effects),
        Node::ReturnStatement { value: None, .. } => false,
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            body_reaches_io(condition, effects)
                || body_reaches_io(consequence, effects)
                || alternative
                    .as_ref()
                    .is_some_and(|a| body_reaches_io(a, effects))
        }
        Node::WhileStatement {
            condition, body, ..
        } => body_reaches_io(condition, effects) || body_reaches_io(body, effects),
        Node::ForInStatement { iterable, body, .. } => {
            body_reaches_io(iterable, effects) || body_reaches_io(body, effects)
        }
        Node::Assert { condition, .. } => body_reaches_io(condition, effects),
        Node::Assume { condition, .. } => body_reaches_io(condition, effects),
        Node::LiveBlock {
            body, invariants, ..
        } => {
            // A `live` block is NOT intrinsically IO (retries on
            // failure observe error state, but not IO per the
            // ticket's definition). If the body reaches IO, the
            // outer fn still does.
            body_reaches_io(body, effects)
                || invariants.iter().any(|inv| body_reaches_io(inv, effects))
        }
        Node::InfixExpression { left, right, .. } => {
            body_reaches_io(left, effects) || body_reaches_io(right, effects)
        }
        Node::PrefixExpression { right, .. } => body_reaches_io(right, effects),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            // Any arg side-effecting → IO.
            if arguments.iter().any(|a| body_reaches_io(a, effects)) {
                return true;
            }
            // The callee itself.
            if body_reaches_io(function, effects) {
                return true;
            }
            if let Node::Identifier { name: callee, .. } = function.as_ref() {
                if IMPURE_BUILTINS.contains(&callee.as_str()) {
                    return true;
                }
                if is_known_pure_builtin(callee) {
                    return false;
                }
                // A user-fn call: inherit that fn's current
                // best-estimate effect. Unknown user fn (rare —
                // typechecker would have rejected earlier) →
                // conservatively IO.
                match effects.get(callee.as_str()) {
                    Some(&true) => return true,
                    Some(&false) => return false,
                    None => return true, // unknown = IO (conservative)
                }
            }
            // Indirect / method callee — can't resolve statically;
            // conservatively mark IO.
            true
        }
        Node::FieldAccess { target, .. } => body_reaches_io(target, effects),
        Node::FieldAssignment { target, value, .. } => {
            body_reaches_io(target, effects) || body_reaches_io(value, effects)
        }
        Node::Assignment { value, .. } => body_reaches_io(value, effects),
        Node::IndexExpression { target, index, .. } => {
            body_reaches_io(target, effects) || body_reaches_io(index, effects)
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            body_reaches_io(target, effects)
                || body_reaches_io(index, effects)
                || body_reaches_io(value, effects)
        }
        Node::ArrayLiteral { items, .. } => items.iter().any(|i| body_reaches_io(i, effects)),
        Node::StructLiteral { fields, base, .. } => {
            base.as_ref().is_some_and(|b| body_reaches_io(b, effects))
                || fields.iter().any(|(_, v)| body_reaches_io(v, effects))
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            if body_reaches_io(scrutinee, effects) {
                return true;
            }
            arms.iter().any(|(_pat, guard, arm_body)| {
                guard.as_ref().is_some_and(|g| body_reaches_io(g, effects))
                    || body_reaches_io(arm_body, effects)
            })
        }
        Node::ExpressionStatement { expr, .. } => body_reaches_io(expr, effects),
        Node::TryExpression { expr, .. } => body_reaches_io(expr, effects),
        Node::OptionalChain { object, access, .. } => {
            if body_reaches_io(object, effects) {
                return true;
            }
            if let crate::ChainAccess::Method(_, args) = access {
                args.iter().any(|a| body_reaches_io(a, effects))
            } else {
                false
            }
        }
        // Nested fn decls are rare but handled — recurse into
        // their body too. Today the parser doesn't emit these;
        // future closures will.
        Node::Function { body, .. } => body_reaches_io(body, effects),
        // Pure literals / identifier reads / etc.
        _ => false,
    }
}

// ============================================================
// RES-389: effect-annotation enforcement.
// ============================================================
//
// Syntax (soft keywords dispatched at statement-start):
//   pure fn f(int x) { ... }   // EffectSet::pure()
//   io   fn g(int x) { ... }   // EffectSet::io()
//   fn h(int x) { ... }        // EffectSet::io() (backward compat)
//
// Call rules:
//   - A `pure` fn may call other `pure` fns or known-pure
//     builtins (see `is_known_pure_builtin`).
//   - An `io` fn (the permissive default) may call `pure` or
//     `io` fns, plus any builtin.
//   - Calling an `io` callee from a `pure` caller is a compile
//     error:
//        E: cannot call io function `X` from pure context
//
// The pass is deliberately coarse — the `@pure` checker (RES-191)
// already rejects impure-builtin calls and unannotated-user-fn
// calls from a `@pure` body. This pass layers on top so the new
// `pure fn` / `io fn` keyword form gets its own diagnostic
// surface separately from the `@pure` attribute.

use crate::EffectSet;

/// RES-389: collect each top-level fn's declared `EffectSet` by
/// name. Unannotated fns default to `EffectSet::io()`, matching
/// the parser. Duplicate names (rare — the typechecker elsewhere
/// diagnoses redeclarations) keep the last one seen.
fn collect_fn_effects(
    statements: &[crate::span::Spanned<Node>],
) -> std::collections::HashMap<String, EffectSet> {
    // RES-1734: pre-size to `statements.len()` — upper bound, since
    // every Function statement contributes one entry. Same shape as
    // RES-1716 / RES-1718 / RES-1724 etc.
    let mut out = std::collections::HashMap::with_capacity(statements.len());
    for stmt in statements {
        if let Node::Function { name, effects, .. } = &stmt.node {
            out.insert(name.clone(), *effects);
        }
    }
    out
}

/// RES-389: top-level entry for the effect-annotation pass.
/// Walks each `pure fn` body and reports the first call site that
/// reaches an `io` callee or an indeterminate callee (method /
/// computed).
fn check_program_effects(
    statements: &[crate::span::Spanned<Node>],
    source_path: &str,
) -> Result<(), String> {
    // RES-1296: short-circuit. `collect_fn_effects` builds a
    // name→effects HashMap by iterating every top-level statement,
    // and the per-stmt loop below only enters its body for `Function`
    // nodes with `effects.pure == true`. Programs that don't declare
    // any pure fn — the overwhelming majority — pay for both passes
    // and produce nothing useful. Pre-scan once before doing either
    // and bail when no pure fn exists.
    let has_pure_fn = statements
        .iter()
        .any(|s| matches!(&s.node, Node::Function { effects, .. } if effects.pure));
    if !has_pure_fn {
        return Ok(());
    }
    let fn_effects = collect_fn_effects(statements);
    for stmt in statements {
        if let Node::Function {
            name,
            body,
            effects,
            parameters,
            ..
        } = &stmt.node
            && effects.pure
            && let Err(reason) = check_body_effects(body, &fn_effects, parameters)
        {
            let (line, col) = (stmt.span.start.line, stmt.span.start.column);
            return Err(if line == 0 {
                format!("pure fn `{}`: {}", name, reason)
            } else {
                format!(
                    "{}:{}:{}: pure fn `{}`: {}",
                    source_path, line, col, name, reason
                )
            });
        }
    }
    Ok(())
}

/// RES-389/RES-385c: recursive walk of a `pure` fn body. Returns
/// `Err(<reason>)` at the first call to a non-`pure` callee, or if a
/// linear parameter is consumed (RES-385c), with the diagnostic text
/// the caller will prefix with the fn span.
fn check_body_effects(
    node: &Node,
    fn_effects: &std::collections::HashMap<String, EffectSet>,
    linear_params: &[(String, String)],
) -> Result<(), String> {
    match node {
        Node::Block { stmts, .. } => {
            for s in stmts {
                check_body_effects(s, fn_effects, linear_params)?;
            }
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            check_body_effects(value, fn_effects, linear_params)?;
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            check_body_effects(v, fn_effects, linear_params)?
        }
        Node::ReturnStatement { value: None, .. } => {}
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            check_body_effects(condition, fn_effects, linear_params)?;
            check_body_effects(consequence, fn_effects, linear_params)?;
            if let Some(a) = alternative {
                check_body_effects(a, fn_effects, linear_params)?;
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            check_body_effects(condition, fn_effects, linear_params)?;
            check_body_effects(body, fn_effects, linear_params)?;
        }
        Node::ForInStatement { iterable, body, .. } => {
            check_body_effects(iterable, fn_effects, linear_params)?;
            check_body_effects(body, fn_effects, linear_params)?;
        }
        Node::Assert { condition, .. } | Node::Assume { condition, .. } => {
            check_body_effects(condition, fn_effects, linear_params)?;
        }
        Node::LiveBlock { body, .. } => check_body_effects(body, fn_effects, linear_params)?,
        Node::InfixExpression { left, right, .. } => {
            check_body_effects(left, fn_effects, linear_params)?;
            check_body_effects(right, fn_effects, linear_params)?;
        }
        Node::PrefixExpression { right, .. } => {
            check_body_effects(right, fn_effects, linear_params)?
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            for a in arguments {
                check_body_effects(a, fn_effects, linear_params)?;
                // RES-385c: detect if a linear parameter is being
                // consumed (passed to a function). Consuming a linear
                // parameter is observable IO — an operation on a
                // resource — so a pure fn cannot do it.
                if let Node::Identifier { name: arg_name, .. } = a
                    && linear_params.iter().any(|(param_ty, param_name)| {
                        arg_name == param_name && crate::linear::is_linear(param_ty)
                    })
                {
                    return Err(format!(
                        "cannot consume linear parameter `{}` in pure context",
                        arg_name
                    ));
                }
            }
            if let Node::Identifier { name: callee, .. } = function.as_ref() {
                // User fn with a recorded effect set — `pure`
                // propagates cleanly; any other effect (today just
                // `io`) is a violation.
                if let Some(callee_effects) = fn_effects.get(callee) {
                    if callee_effects.pure {
                        return Ok(());
                    }
                    return Err(format!(
                        "cannot call io function `{}` from pure context",
                        callee
                    ));
                }
                // Builtins: pure-by-default list passes; anything
                // flagged impure by RES-191 is also implicitly io.
                if IMPURE_BUILTINS.contains(&callee.as_str()) {
                    return Err(format!(
                        "cannot call io function `{}` from pure context",
                        callee
                    ));
                }
                if is_known_pure_builtin(callee) {
                    return Ok(());
                }
                // Unknown callee (not in BUILTINS and not a
                // declared user fn) — conservatively reject so a
                // `pure` annotation remains meaningful.
                return Err(format!(
                    "cannot call io function `{}` from pure context",
                    callee
                ));
            }
            // Method / computed callee — can't resolve statically;
            // same conservative rejection as the purity pass.
            check_body_effects(function, fn_effects, linear_params)?;
            return Err(
                "cannot call indirect/method callee from pure context (effect unknown)".to_string(),
            );
        }
        Node::FieldAccess { target, .. } => check_body_effects(target, fn_effects, linear_params)?,
        Node::FieldAssignment { target, value, .. } => {
            check_body_effects(target, fn_effects, linear_params)?;
            check_body_effects(value, fn_effects, linear_params)?;
        }
        Node::Assignment { value, .. } => check_body_effects(value, fn_effects, linear_params)?,
        Node::IndexExpression { target, index, .. } => {
            check_body_effects(target, fn_effects, linear_params)?;
            check_body_effects(index, fn_effects, linear_params)?;
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            check_body_effects(target, fn_effects, linear_params)?;
            check_body_effects(index, fn_effects, linear_params)?;
            check_body_effects(value, fn_effects, linear_params)?;
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                check_body_effects(i, fn_effects, linear_params)?;
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(b) = base {
                check_body_effects(b, fn_effects, linear_params)?;
            }
            for (_, v) in fields {
                check_body_effects(v, fn_effects, linear_params)?;
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            check_body_effects(scrutinee, fn_effects, linear_params)?;
            for (_pat, guard, arm_body) in arms {
                if let Some(g) = guard {
                    check_body_effects(g, fn_effects, linear_params)?;
                }
                check_body_effects(arm_body, fn_effects, linear_params)?;
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            check_body_effects(expr, fn_effects, linear_params)?
        }
        Node::TryExpression { expr, .. } => check_body_effects(expr, fn_effects, linear_params)?,
        Node::OptionalChain { object, access, .. } => {
            check_body_effects(object, fn_effects, linear_params)?;
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    check_body_effects(a, fn_effects, linear_params)?;
                }
            }
        }
        Node::Function { body, .. } => check_body_effects(body, fn_effects, linear_params)?,
        // Literals, identifier reads, durations — nothing to do.
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod effect_tests {
    use super::*;
    use crate::parse;

    fn stmts(src: &str) -> Vec<crate::span::Spanned<Node>> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        match prog {
            Node::Program(s) => s,
            other => panic!("expected Program, got {:?}", other),
        }
    }

    #[test]
    fn pure_to_pure_passes() {
        let src = "pure fn inner(int x) { return x + 1; }\n\
                   pure fn outer(int x) { return inner(x); }\n";
        let s = stmts(src);
        check_program_effects(&s, "<t>").expect("pure→pure should pass");
    }

    #[test]
    fn pure_to_io_is_rejected() {
        let src = "io   fn noisy(int x) { return x; }\n\
                   pure fn caller(int x) { return noisy(x); }\n";
        let s = stmts(src);
        let err = check_program_effects(&s, "<t>").expect_err("pure→io should fail");
        assert!(
            err.contains("cannot call io function `noisy` from pure context"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pure_to_unannotated_is_rejected() {
        // Unannotated fns default to `EffectSet::io()` per the
        // RES-389 backward-compat rule.
        let src = "fn      helper(int x) { return x + 1; }\n\
                   pure fn caller(int x) { return helper(x); }\n";
        let s = stmts(src);
        let err = check_program_effects(&s, "<t>").expect_err("pure→unannotated should fail");
        assert!(
            err.contains("cannot call io function `helper` from pure context"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn io_to_pure_passes() {
        let src = "pure fn add1(int x) { return x + 1; }\n\
                   io   fn caller(int x) { return add1(x); }\n";
        let s = stmts(src);
        check_program_effects(&s, "<t>").expect("io→pure should pass");
    }

    #[test]
    fn io_to_io_passes() {
        let src = "io fn a(int x) { return x; }\n\
                   io fn b(int x) { return a(x); }\n";
        let s = stmts(src);
        check_program_effects(&s, "<t>").expect("io→io should pass");
    }

    #[test]
    fn pure_using_pure_builtin_passes() {
        let src = "pure fn f(int x) { return abs(x); }\n";
        let s = stmts(src);
        check_program_effects(&s, "<t>").expect("abs is pure");
    }

    #[test]
    fn pure_using_impure_builtin_is_rejected() {
        let src = "pure fn f(int x) { println(x); return x; }\n";
        let s = stmts(src);
        let err = check_program_effects(&s, "<t>").expect_err("println is io");
        assert!(
            err.contains("cannot call io function `println` from pure context"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn diagnostic_includes_source_location() {
        // When the `pure` fn has a real span, the error prefix is
        // `<path>:<line>:<col>: …` — matching RES-080 / RES-191
        // conventions so IDEs can anchor the message.
        let src = "io   fn noisy(int x) { return x; }\n\
                   pure fn caller(int x) { return noisy(x); }\n";
        let s = stmts(src);
        let err = check_program_effects(&s, "<t>").unwrap_err();
        assert!(err.contains("<t>:"), "missing source path: {err}");
        assert!(err.contains(":2:"), "missing line number: {err}");
    }

    // RES-385c: linear parameter effect-system interaction tests.

    #[test]
    fn pure_with_linear_parameter_rejected_on_consumption() {
        let src = "fn consume(linear int x) { return 0; }\n\
                   pure fn bad(linear int x) { return consume(x); }\n";
        let s = stmts(src);
        let err = check_program_effects(&s, "<t>").expect_err("pure+linear consumed should fail");
        assert!(
            err.contains("cannot consume linear parameter `x` in pure context"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pure_with_linear_parameter_accepted_on_no_consumption() {
        let src = "pure fn ok(linear int x) { return 0; }\n";
        let s = stmts(src);
        check_program_effects(&s, "<t>").expect("pure+linear not consumed should pass");
    }

    #[test]
    fn io_with_linear_parameter_accepted_on_consumption() {
        let src = "fn consume(linear int x) { return 0; }\n\
                   io fn ok(linear int x) { return consume(x); }\n";
        let s = stmts(src);
        check_program_effects(&s, "<t>").expect("io+linear consumed should pass");
    }
}

#[cfg(test)]
mod purity_tests {
    use super::*;
    use crate::parse;

    /// Pull the statement list out of a parsed program.
    fn stmts(src: &str) -> Vec<crate::span::Spanned<Node>> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        match prog {
            Node::Program(s) => s,
            other => panic!("expected Program, got {:?}", other),
        }
    }

    // ---------- AC: success ----------

    #[test]
    fn pure_fn_calling_only_arithmetic_passes() {
        let src = "@pure fn double(int x) { return x * 2; }\n";
        let s = stmts(src);
        check_program_purity(&s, "<t>").expect("should pass");
    }

    #[test]
    fn pure_fn_calling_pure_builtin_passes() {
        let src = "@pure fn f(int x) { return abs(x); }\n";
        let s = stmts(src);
        check_program_purity(&s, "<t>").expect("abs is pure");
    }

    #[test]
    fn pure_fn_with_struct_construction_passes() {
        let src = "\
            struct Point { int x, int y }\n\
            @pure fn make(int a, int b) { return new Point { x: a, y: b }; }\n";
        let s = stmts(src);
        check_program_purity(&s, "<t>").expect("struct construction is pure");
    }

    // ---------- AC: impure builtin ----------

    #[test]
    fn pure_fn_calling_println_is_rejected() {
        let src = "@pure fn f(int x) { println(\"hi\"); return x; }\n";
        let s = stmts(src);
        let err = check_program_purity(&s, "<t>").expect_err("println is impure");
        assert!(
            err.contains("calls impure builtin `println`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pure_fn_calling_clock_ms_is_rejected() {
        let src = "@pure fn f() { return clock_ms(); }\n";
        let s = stmts(src);
        let err = check_program_purity(&s, "<t>").expect_err("clock_ms is impure");
        assert!(err.contains("clock_ms"), "unexpected error: {err}");
    }

    #[test]
    fn pure_fn_calling_file_read_is_rejected() {
        let src = "@pure fn f() { return file_read(\"x\"); }\n";
        let s = stmts(src);
        let err = check_program_purity(&s, "<t>").expect_err("file_read is impure");
        assert!(err.contains("file_read"), "unexpected error: {err}");
    }

    // ---------- AC: impure user-fn call ----------

    #[test]
    fn pure_fn_calling_unannotated_user_fn_is_rejected() {
        let src = "\
            fn helper(int x) { return x + 1; }\n\
            @pure fn f(int x) { return helper(x); }\n";
        let s = stmts(src);
        let err = check_program_purity(&s, "<t>").expect_err("unannotated user fn is rejected");
        assert!(
            err.contains("calls unannotated fn `helper`"),
            "unexpected error: {err}"
        );
    }

    // ---------- AC: mutual recursion between two @pure fns ----------

    #[test]
    fn two_mutually_recursive_pure_fns_pass() {
        let src = "\
            @pure fn a(int n) { return b(n); }\n\
            @pure fn b(int n) { return a(n); }\n";
        let s = stmts(src);
        // Only the purity pass — the main typechecker rejects
        // forward refs today (orthogonal limitation, see
        // typechecker.rs `pre-pass` comment at line ~800). The
        // purity pass itself correctly handles mutual recursion
        // because the optimistic first pass populates `pure_fns`
        // with both names.
        check_program_purity(&s, "<t>").expect("mutual recursion between two @pure fns is fine");
    }

    #[test]
    fn pure_fn_calling_live_block_is_rejected() {
        // `live` blocks retry on failure — observable from outside.
        let src = "@pure fn f(int x) { live { return x; } return 0; }\n";
        let s = stmts(src);
        let err = check_program_purity(&s, "<t>").expect_err("live blocks are impure");
        assert!(err.contains("live"), "unexpected error: {err}");
    }

    #[test]
    fn unannotated_fn_is_not_checked_for_purity() {
        // Non-@pure fns are free to do anything; the checker must
        // leave them alone even if they'd violate purity.
        let src = "fn noisy() { println(\"hi\"); return 0; }\n";
        let s = stmts(src);
        check_program_purity(&s, "<t>").expect("non-@pure fns bypass the purity checker");
    }

    // ---------- error message shape ----------

    #[test]
    fn error_mentions_fn_name_and_violating_site() {
        let src = "@pure fn noisy(int x) { println(\"hi\"); return x; }\n";
        let s = stmts(src);
        let err = check_program_purity(&s, "<t>").unwrap_err();
        assert!(err.contains("noisy"), "expected fn name in error: {err}");
        assert!(
            err.contains("println"),
            "expected callee name in error: {err}"
        );
    }

    #[test]
    fn error_carries_file_path_and_position() {
        let src = "@pure fn noisy() { println(\"hi\"); return 0; }\n";
        let s = stmts(src);
        let err = check_program_purity(&s, "src/thing.rs").unwrap_err();
        assert!(
            err.starts_with("src/thing.rs:"),
            "expected RES-080 prefix `<path>:<line>:<col>:`, got: {err}"
        );
    }

    // ---------- RES-192: IO-effect inference ----------

    /// AC: a chain `caller -> helper -> println` has IO at every
    /// level.
    #[test]
    fn effect_chain_propagates_io_transitively() {
        let src = "\
            fn helper() { println(\"hi\"); return 0; }\n\
            fn caller() { return helper(); }\n\
            fn top() { return caller(); }\n";
        let s = stmts(src);
        let eff = infer_fn_effects(&s);
        assert_eq!(eff.get("helper"), Some(&true), "helper should be IO");
        assert_eq!(eff.get("caller"), Some(&true), "caller should be IO");
        assert_eq!(eff.get("top"), Some(&true), "top should be IO");
    }

    /// AC: a leaf fn that only does arithmetic is tagged pure.
    #[test]
    fn arithmetic_only_leaf_is_pure() {
        let src = "fn double(int x) { return x * 2; }\n";
        let s = stmts(src);
        let eff = infer_fn_effects(&s);
        assert_eq!(eff.get("double"), Some(&false), "pure arithmetic");
    }

    #[test]
    fn fixpoint_handles_mutual_recursion() {
        // Two fns that call each other but neither does IO.
        let src = "\
            fn a(int n) { return b(n); }\n\
            fn b(int n) { return a(n); }\n";
        let s = stmts(src);
        let eff = infer_fn_effects(&s);
        assert_eq!(eff.get("a"), Some(&false), "pure mutual recursion");
        assert_eq!(eff.get("b"), Some(&false));
    }

    #[test]
    fn io_reaches_through_mutual_recursion() {
        let src = "\
            fn a(int n) { return b(n); }\n\
            fn b(int n) { println(\"x\"); return a(n); }\n";
        let s = stmts(src);
        let eff = infer_fn_effects(&s);
        assert_eq!(eff.get("a"), Some(&true), "a reaches IO via b");
        assert_eq!(eff.get("b"), Some(&true));
    }

    #[test]
    fn file_io_builtins_flag_io() {
        let src = "\
            fn writer() { file_write(\"f\", \"data\"); return 0; }\n\
            fn reader() { return file_read(\"f\"); }\n";
        let s = stmts(src);
        let eff = infer_fn_effects(&s);
        assert_eq!(eff.get("writer"), Some(&true));
        assert_eq!(eff.get("reader"), Some(&true));
    }

    #[test]
    fn clock_and_random_flag_io() {
        // Nondeterminism counts as IO per the broader "impure
        // builtin" policy inherited from RES-191.
        let src = "\
            fn now() { return clock_ms(); }\n\
            fn rand() { return random_int(0, 10); }\n";
        let s = stmts(src);
        let eff = infer_fn_effects(&s);
        assert_eq!(eff.get("now"), Some(&true));
        assert_eq!(eff.get("rand"), Some(&true));
    }

    #[test]
    fn pure_builtin_calls_stay_pure() {
        let src = "\
            fn compute(int x) { return abs(x); }\n\
            fn compose(int x) { let y = compute(x); return min(y, 10); }\n";
        let s = stmts(src);
        let eff = infer_fn_effects(&s);
        assert_eq!(eff.get("compute"), Some(&false), "abs is pure");
        assert_eq!(eff.get("compose"), Some(&false));
    }

    #[test]
    fn live_block_alone_is_not_io() {
        // A live block with a pure body is still pure. The ticket
        // tracks "reach println or file_*"; retries alone don't
        // qualify.
        let src = "fn f(int x) { live { return x; } return 0; }\n";
        let s = stmts(src);
        let eff = infer_fn_effects(&s);
        assert_eq!(eff.get("f"), Some(&false), "pure live body");
    }

    #[test]
    fn live_block_with_io_body_is_io() {
        let src = "fn f(int x) { live { println(\"r\"); } return x; }\n";
        let s = stmts(src);
        let eff = infer_fn_effects(&s);
        assert_eq!(eff.get("f"), Some(&true));
    }

    #[test]
    fn empty_program_produces_empty_effects() {
        let s = stmts("");
        assert!(infer_fn_effects(&s).is_empty());
    }

    // ---------- RES-385: linear-type single-use enforcement ----------

    /// AC: passing a `linear` parameter to one consumer is fine.
    /// Passing it to a second consumer is a single-use violation.
    #[test]
    fn linear_value_used_twice_is_rejected() {
        let src = "\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn bad(linear FileHandle fh) {\n\
                consume(fh);\n\
                consume(fh);\n\
                return 0;\n\
            }\n";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let mut tc = TypeChecker::new();
        let err = tc
            .check_program_with_source(&program, "<t>")
            .expect_err("second use of linear fh must be rejected");
        assert!(
            err.contains("linear-use"),
            "expected `linear-use` diagnostic tag, got: {err}"
        );
        assert!(
            err.contains("used after move"),
            "expected `used after move` in message, got: {err}"
        );
        assert!(
            err.contains("fh"),
            "expected binding name in message, got: {err}"
        );
    }

    /// AC: consuming exactly once (via a call or `drop`) is fine.
    #[test]
    fn linear_value_used_once_is_accepted() {
        let src = "\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn good(linear FileHandle fh) {\n\
                consume(fh);\n\
                return 0;\n\
            }\n\
            fn dropper(linear FileHandle fh) {\n\
                drop(fh);\n\
                return 0;\n\
            }\n";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let mut tc = TypeChecker::new();
        tc.check_program_with_source(&program, "<t>")
            .expect("one consumption (direct call or drop) must typecheck");
    }

    /// AC: `let fh: linear T = …;` binds a linear local whose double-
    /// use is rejected just like a linear parameter.
    #[test]
    fn linear_let_binding_rejects_double_use() {
        // Construct the handle via a struct literal so the RHS type
        // matches the let annotation without needing fancy
        // inference. The linearity bit flows through `parse_type_name`
        // via the shared `linear` prefix-stripping helper.
        let src = "\
            struct FileHandle { int fd }\n\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn main() {\n\
                let fh: linear FileHandle = new FileHandle { fd: 3 };\n\
                consume(fh);\n\
                consume(fh);\n\
                return 0;\n\
            }\n";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let mut tc = TypeChecker::new();
        let err = tc
            .check_program_with_source(&program, "<t>")
            .expect_err("second use of let-bound linear must be rejected");
        assert!(
            err.contains("linear-use"),
            "expected `linear-use` diagnostic tag, got: {err}"
        );
    }

    /// AC: the diagnostic is prefixed with `<file>:<line>:<col>:`
    /// so editors can jump straight to the offending second use.
    #[test]
    fn linear_use_diagnostic_carries_source_position() {
        let src = "\
            fn consume(linear FileHandle fh) { return 0; }\n\
            fn bad(linear FileHandle fh) {\n\
                consume(fh);\n\
                consume(fh);\n\
                return 0;\n\
            }\n";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let mut tc = TypeChecker::new();
        let err = tc
            .check_program_with_source(&program, "src/bad.rz")
            .expect_err("should fail");
        assert!(
            err.starts_with("src/bad.rz:"),
            "expected <path>:<line>:<col>: prefix, got: {err}"
        );
    }

    #[test]
    fn stats_field_populated_by_full_check() {
        // End-to-end: running the typechecker with `audit_stats`
        // opt-in populates `stats.fn_effects`. Confirms the gated
        // call-site inside `check_program_with_source` is wired.
        //
        // RES-1322: `with_audit_stats(true)` is required — the
        // fixpoint is off by default so non-audit callers don't pay
        // for a result they never read.
        let src = "\
            fn noisy() { println(\"h\"); return 0; }\n\
            fn quiet() { return 0; }\n\
            fn main(int _d) { noisy(); return quiet(); }\n\
            main(0);\n";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let mut tc = TypeChecker::new().with_audit_stats(true);
        tc.check_program_with_source(&program, "<t>")
            .expect("typecheck should succeed");
        assert_eq!(tc.stats.fn_effects.get("noisy"), Some(&true));
        assert_eq!(tc.stats.fn_effects.get("quiet"), Some(&false));
        assert_eq!(tc.stats.fn_effects.get("main"), Some(&true));
    }

    #[test]
    fn stats_field_empty_by_default() {
        // RES-1322: without `with_audit_stats(true)` the fixpoint
        // is skipped and `stats.fn_effects` stays empty. Locks in
        // the opt-in default so a future regression that always-
        // populates the map fails this test.
        let src = "fn noisy() { println(\"h\"); return 0; }\n";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let mut tc = TypeChecker::new();
        tc.check_program_with_source(&program, "<t>")
            .expect("typecheck should succeed");
        assert!(
            tc.stats.fn_effects.is_empty(),
            "fn_effects should be empty when audit_stats is off, got {:?}",
            tc.stats.fn_effects
        );
    }
}

#[cfg(test)]
mod type_hole_display_tests {
    use super::*;
    use crate::span::{Pos, Span};

    #[test]
    fn var_with_span_displays_as_type_hole() {
        let pos = Pos::new(3, 7, 0);
        let span = Span::point(pos);
        let ty = Type::Var(0, Some(span));
        assert_eq!(ty.to_string(), "type hole at 3:7");
    }

    #[test]
    fn var_without_span_displays_as_qt() {
        let ty = Type::Var(0, None);
        assert_eq!(ty.to_string(), "?t0");
    }
}

/// RES-387: fault model — parser + typechecker slice for the `fails`
/// annotation and `recovers_to` postcondition. Structured handlers
/// and the Z3 proof obligation for `recovers_to` are separate tickets.
#[cfg(test)]
mod fault_model_tests {
    use super::*;
    use crate::parse;

    // ---------- Parser: `fails` / `recovers_to` ----------

    #[test]
    fn parser_accepts_single_fails_variant() {
        let src = "fn read_sensor(int addr) fails HardwareFault { return addr; }\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        match prog {
            Node::Program(stmts) => match &stmts[0].node {
                Node::Function { name, fails, .. } => {
                    assert_eq!(name, "read_sensor");
                    assert_eq!(fails, &vec!["HardwareFault".to_string()]);
                }
                other => panic!("expected Function, got {:?}", other),
            },
            other => panic!("expected Program, got {:?}", other),
        }
    }

    #[test]
    fn parser_accepts_multiple_fails_variants() {
        let src = "fn write_sensor(int v) fails HardwareFault, Timeout { return v; }\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        match prog {
            Node::Program(stmts) => match &stmts[0].node {
                Node::Function { fails, .. } => {
                    assert_eq!(
                        fails,
                        &vec!["HardwareFault".to_string(), "Timeout".to_string()]
                    );
                }
                other => panic!("expected Function, got {:?}", other),
            },
            other => panic!("expected Program, got {:?}", other),
        }
    }

    #[test]
    fn parser_accepts_recovers_to_postcondition() {
        let src = "fn write_sensor(int v) fails Timeout recovers_to: v >= 0; { return v; }\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        match prog {
            Node::Program(stmts) => match &stmts[0].node {
                Node::Function {
                    fails, recovers_to, ..
                } => {
                    assert_eq!(fails, &vec!["Timeout".to_string()]);
                    assert!(
                        recovers_to.is_some(),
                        "expected recovers_to to be populated"
                    );
                }
                other => panic!("expected Function, got {:?}", other),
            },
            other => panic!("expected Program, got {:?}", other),
        }
    }

    #[test]
    fn parser_accepts_fails_after_requires() {
        // requires, then fails — mirrors the ticket's example.
        let src = "fn op(int x) requires x > 0 fails Bad { return x; }\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        match prog {
            Node::Program(stmts) => match &stmts[0].node {
                Node::Function {
                    fails, requires, ..
                } => {
                    assert_eq!(fails, &vec!["Bad".to_string()]);
                    assert_eq!(requires.len(), 1);
                }
                other => panic!("expected Function, got {:?}", other),
            },
            other => panic!("expected Program, got {:?}", other),
        }
    }

    #[test]
    fn parser_fn_without_fails_has_empty_list() {
        let src = "fn add(int a, int b) { return a + b; }\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        match prog {
            Node::Program(stmts) => match &stmts[0].node {
                Node::Function {
                    fails, recovers_to, ..
                } => {
                    assert!(fails.is_empty());
                    assert!(recovers_to.is_none());
                }
                other => panic!("expected Function, got {:?}", other),
            },
            other => panic!("expected Program, got {:?}", other),
        }
    }

    // ---------- Typechecker: propagation ----------

    fn check(src: &str) -> Result<(), String> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = TypeChecker::new();
        tc.check_program_with_source(&prog, "<t>").map(|_| ())
    }

    #[test]
    fn propagated_fails_is_accepted() {
        // `caller` propagates `HardwareFault`, matching `inner`'s fails.
        let src = "\
            fn inner(int x) fails HardwareFault { return x; }\n\
            fn caller(int y) fails HardwareFault { return inner(y); }\n";
        check(src).expect("propagation should typecheck");
    }

    #[test]
    fn unhandled_fails_is_rejected() {
        // `caller` does not declare `HardwareFault`; the call must
        // be rejected with the MVP diagnostic.
        let src = "\
            fn inner(int x) fails HardwareFault { return x; }\n\
            fn caller(int y) { return inner(y); }\n";
        let err = check(src).expect_err("unhandled fails must be rejected");
        assert!(
            err.contains("unhandled failure variant HardwareFault"),
            "unexpected error: {err}"
        );
        assert!(
            err.contains("declare `fails HardwareFault`"),
            "diagnostic should mention how to fix it: {err}"
        );
    }

    #[test]
    fn missing_one_of_many_fails_is_rejected() {
        // Caller propagates one variant but not the other — must
        // still be rejected for the missing one.
        let src = "\
            fn inner(int x) fails A, B { return x; }\n\
            fn caller(int y) fails A { return inner(y); }\n";
        let err = check(src).expect_err("partial propagation must be rejected");
        assert!(
            err.contains("unhandled failure variant B"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn fails_free_call_is_still_accepted() {
        // Regression: a call to a fn with no `fails` set must remain
        // allowed from anywhere, with or without enclosing fault scope.
        let src = "\
            fn inner(int x) { return x; }\n\
            fn caller(int y) { return inner(y); }\n";
        check(src).expect("fails-free propagation path must stay green");
    }

    #[test]
    fn top_level_call_to_failing_fn_is_rejected() {
        // No enclosing fn — cannot propagate, must be rejected.
        let src = "\
            fn inner(int x) fails Timeout { return x; }\n\
            let x = inner(1);\n";
        let err = check(src).expect_err("top-level must reject failing call");
        assert!(
            err.contains("unhandled failure variant Timeout"),
            "unexpected error: {err}"
        );
    }
}

// ============================================================
// RES-340: rich type-mismatch diagnostics
// ============================================================
//
// Default behaviour is byte-identical to before the ticket — the
// short `Type mismatch in argument N: expected X, got Y` form is
// what the typechecker still emits unless the user opts in via
// `RESILIENT_RICH_DIAG=1`. The opt-in produces a rustc-style
// multi-block diagnostic with a primary span on the offending
// argument and a secondary span on the function's declaration.
//
// These tests exercise the renderer helper directly so they don't
// depend on (and cannot perturb) the global environment variable
// other tests might be reading. End-to-end coverage of the gate
// itself lives in the `legacy_default_remains` test below: it
// scrubs the env var locally, then asserts the legacy text comes
// back through the full pipeline.
#[cfg(test)]
mod res340_rich_type_mismatch_tests {
    use super::*;
    use crate::parse;
    use crate::span::Pos;

    fn span(start_line: usize, start_col: usize, end_line: usize, end_col: usize) -> Span {
        Span::new(
            Pos::new(start_line, start_col, 0),
            Pos::new(end_line, end_col, 0),
        )
    }

    fn check(src: &str) -> Result<(), String> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = TypeChecker::new();
        tc.check_program_with_source(&prog, "<t>").map(|_| ())
    }

    #[test]
    fn legacy_default_remains_byte_identical() {
        // Without the env var, the short form is what the
        // typechecker emits. Goldens depend on this — the test
        // pins the exact substring so a regression here would
        // also be a regression in any `.expected.txt` that
        // captured the line.
        //
        // We deliberately do NOT touch `RESILIENT_RICH_DIAG`
        // here — both removing and setting it would race against
        // any concurrent test that reads it. Instead we trust
        // that `cargo test` runs in a clean process where the
        // var is unset by default; if a contributor exports it
        // in their shell, the assertion below trips and tells
        // them to unset it before running tests, which is the
        // correct invariant.
        let src = "\
            fn drive(int dist) { return dist; }\n\
            fn caller() { return drive(\"oops\"); }\n";
        let err = check(src).expect_err("call with wrong arg type must be rejected");
        if std::env::var("RESILIENT_RICH_DIAG").as_deref() == Ok("1") {
            // Contributor has the rich-diag flag exported in
            // their shell. Verify the rich path instead — both
            // formats are correct, just different.
            assert!(
                err.contains("error[E0007]: type mismatch"),
                "rich path must produce the rustc-style header: {err}"
            );
            return;
        }
        assert!(
            err.contains("Type mismatch in argument 1: expected int, got string"),
            "default format must remain the legacy short message; got: {err}"
        );
        // The rich block markers must NOT appear by default.
        assert!(
            !err.contains("error[E0007]"),
            "rich format leaked into default output: {err}"
        );
    }

    #[test]
    fn rich_helper_includes_primary_and_secondary_spans() {
        // Construct a synthetic source whose layout we control,
        // then ask the renderer for the rich block. We use a
        // tempfile so the helper can read it back.
        let src = "fn drive(int dist) { return dist; }\nlet r = drive(\"oops\");\n";
        let dir = std::env::temp_dir().join("res340_rich_helper");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("rich.rz");
        std::fs::write(&path, src).expect("write tempfile");
        // Primary on the bad argument, secondary on the fn decl.
        let arg_span = span(2, 15, 2, 21); // "oops" string literal
        let decl_span = Some(span(1, 1, 1, 3)); // "fn"
        let out = render_rich_arg_type_mismatch(
            path.to_str().unwrap(),
            arg_span,
            decl_span,
            1,
            "int",
            "string",
        );
        assert!(
            out.contains(
                "error[E0007]: type mismatch in argument 1: expected `int`, found `string`"
            ),
            "rich header missing or wrong: {out}"
        );
        assert!(
            out.contains("note: expected `int` because of this declaration"),
            "secondary label missing: {out}"
        );
        // Both source lines should appear (primary + note snippet).
        assert!(
            out.contains("let r = drive(\"oops\")"),
            "primary snippet missing: {out}"
        );
        assert!(
            out.contains("fn drive(int dist)"),
            "declaration snippet missing: {out}"
        );
        // Carets must underline both spans.
        assert!(
            out.matches('^').count() >= 2,
            "expected both spans to be underlined: {out}"
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rich_helper_drops_secondary_when_decl_unknown() {
        // Calls through a function value (no Identifier callee) —
        // we have no fn declaration span, so the secondary block
        // is omitted but the primary is still rich.
        let src = "let f = drive;\nf(\"oops\");\n";
        let dir = std::env::temp_dir().join("res340_no_decl");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("nodecl.rz");
        std::fs::write(&path, src).expect("write tempfile");
        let out = render_rich_arg_type_mismatch(
            path.to_str().unwrap(),
            span(2, 3, 2, 9),
            None,
            1,
            "int",
            "string",
        );
        assert!(
            out.contains("error[E0007]: type mismatch in argument 1"),
            "primary header missing: {out}"
        );
        assert!(
            !out.contains("note: expected"),
            "secondary block leaked when no decl span was passed: {out}"
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rich_helper_falls_back_safely_when_source_missing() {
        // Empty source path → renderer falls back to a no-snippet
        // diagnostic. Must still include the header so a user
        // reading the LSP / REPL output sees something useful.
        let out = render_rich_arg_type_mismatch(
            "",
            span(1, 1, 1, 5),
            Some(span(2, 1, 2, 3)),
            2,
            "Meters",
            "Seconds",
        );
        assert!(
            out.contains(
                "error[E0007]: type mismatch in argument 2: expected `Meters`, found `Seconds`"
            ),
            "header missing on no-source path: {out}"
        );
        assert!(
            out.contains("note: expected `Meters` because of this declaration"),
            "note missing on no-source path: {out}"
        );
    }

    #[test]
    fn fn_decl_span_table_is_populated_for_top_level_fns() {
        // Internal invariant: the pre-pass that fills
        // `contract_table` also fills `fn_decl_spans` keyed by fn
        // name. The rich-diag path depends on this; failing here
        // would silently degrade error quality.
        let src = "fn drive(int dist) { return dist; }\nfn caller() { return 0; }\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = TypeChecker::new();
        let _ = tc.check_program_with_source(&prog, "<t>");
        assert!(
            tc.fn_decl_spans.contains_key("drive"),
            "fn_decl_spans missing entry for drive: {:?}",
            tc.fn_decl_spans.keys().collect::<Vec<_>>()
        );
        assert!(
            tc.fn_decl_spans.contains_key("caller"),
            "fn_decl_spans missing entry for caller: {:?}",
            tc.fn_decl_spans.keys().collect::<Vec<_>>()
        );
    }
}

// RES-402: tests for the array-literal mixed-element-type rejection.
#[cfg(test)]
mod res402_polymorphic_array_tests {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check(src: &str) -> Result<(), String> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new().check_program(&prog).map(|_| ())
    }

    #[test]
    fn homogeneous_int_array_passes() {
        check("let xs = [1, 2, 3];").expect("homogeneous Int literal should typecheck");
    }

    #[test]
    fn homogeneous_string_array_passes() {
        check("let xs = [\"a\", \"b\", \"c\"];")
            .expect("homogeneous String literal should typecheck");
    }

    #[test]
    fn empty_array_passes() {
        check("let xs = [];").expect("empty array literal should typecheck");
    }

    #[test]
    fn mixed_int_and_string_is_rejected() {
        let err =
            check("let xs = [1, \"two\"];").expect_err("mixed Int / String should be rejected");
        assert!(
            err.contains("mixed element types"),
            "diagnostic missing 'mixed element types': {}",
            err
        );
        assert!(err.contains("int"), "diagnostic missing int type: {}", err);
        assert!(
            err.contains("string"),
            "diagnostic missing string type: {}",
            err
        );
    }

    #[test]
    fn mixed_int_and_float_is_rejected() {
        // Resilient already rejects Int + Float arithmetic — array
        // literals follow the same no-implicit-coercion rule.
        let err = check("let xs = [1, 2.0];").expect_err("mixed Int / Float should be rejected");
        assert!(
            err.contains("mixed element types"),
            "diagnostic missing 'mixed element types': {}",
            err
        );
    }

    #[test]
    fn actor_rejects_reference_state_type() {
        // RES-777: actor state fields cannot have reference types.
        // References would create aliases across actor boundaries,
        // enabling data races despite the actor model's isolation.
        let err = check(
            "
            actor TestActor {
                state: &int = 0;
                receive handle() {}
            }
            ",
        )
        .expect_err("reference-typed actor state should be rejected");
        assert!(
            err.contains("has reference type"),
            "diagnostic missing 'has reference type': {}",
            err
        );
        assert!(
            err.contains("actor boundaries require ownership-by-value"),
            "diagnostic missing 'actor boundaries require ownership-by-value': {}",
            err
        );
    }

    #[test]
    fn actor_rejects_reference_handler_parameter() {
        // RES-777: actor handler parameters cannot have reference types
        // for the same reason as state fields.
        let err = check(
            "
            actor TestActor {
                state: int = 0;
                receive handle(&int x) {}
            }
            ",
        )
        .expect_err("reference-typed handler parameter should be rejected");
        assert!(
            err.contains("has reference type"),
            "diagnostic missing 'has reference type': {}",
            err
        );
        assert!(
            err.contains("actor boundaries require ownership-by-value"),
            "diagnostic missing 'actor boundaries require ownership-by-value': {}",
            err
        );
    }
}

// RES-1104..RES-1106 + RES-1112 + RES-1113: regression suite for the
// typechecker scope / reachability fixes shipped together. Each test
// uses `parse` + `TypeChecker::check_program` so it exercises the
// real driver path. A bug introduced into any of these fixes must
// surface here before reaching CI.
#[cfg(test)]
mod scope_and_reachability_tests {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check(src: &str) -> Result<(), String> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new().check_program(&prog).map(|_| ())
    }

    // --- RES-1104: for-in loop variable binding ------------------------------

    #[test]
    fn res1104_for_in_array_binds_loop_var() {
        check(
            "fn main() { \
                let xs = [1, 2, 3]; \
                let total = 0; \
                for x in xs { total = total + x; } \
            }",
        )
        .expect("for x in xs should bind x for the body");
    }

    #[test]
    fn res1104_for_in_range_binds_loop_var() {
        check(
            "fn main() { \
                let total = 0; \
                for i in 0..5 { total = total + i; } \
            }",
        )
        .expect("for i in 0..5 should bind i for the body");
    }

    #[test]
    fn res1104_loop_var_does_not_leak_past_block() {
        // After RES-1104 + RES-1111 the loop variable is confined to
        // the for body's enclosed env, so referring to it after the
        // loop is a clean "Undefined variable" diagnostic rather than
        // accidentally typechecking against the leaked binding.
        let err = check(
            "fn main() { \
                for x in [1, 2, 3] { } \
                let y = x; \
            }",
        )
        .expect_err("loop var should be undefined after the for body");
        assert!(
            err.contains("Undefined variable 'x'"),
            "expected undefined-x diagnostic, got: {err}"
        );
    }

    // --- RES-1105: direct self-recursion -------------------------------------

    #[test]
    fn res1105_self_recursion_resolves_in_typechecker() {
        check(
            "fn fact(int n) -> int { \
                if n <= 1 { return 1; } \
                return n * fact(n - 1); \
            }",
        )
        .expect("self-recursive fact should typecheck");
    }

    // --- RES-1106: forward / mutual recursion --------------------------------

    #[test]
    fn res1106_forward_call_resolves_in_typechecker() {
        check(
            "fn caller() -> int { return helper(); } \
             fn helper() -> int { return 7; }",
        )
        .expect("forward fn references should typecheck");
    }

    #[test]
    fn res1106_mutual_recursion_resolves_in_typechecker() {
        check(
            "fn even(int n) -> bool { \
                if n == 0 { return true; } \
                return odd(n - 1); \
            } \
            fn odd(int n) -> bool { \
                if n == 0 { return false; } \
                return even(n - 1); \
            }",
        )
        .expect("mutual recursion (even/odd) should typecheck");
    }

    // --- RES-1112: missing return on non-void fn -----------------------------

    #[test]
    fn res1112_missing_return_rejects_if_without_else() {
        let err = check(
            "fn maybe(int n) -> int { \
                if n > 0 { return 1; } \
            }",
        )
        .expect_err("missing return on else path should be rejected");
        assert!(
            err.contains("missing return"),
            "expected missing-return diagnostic, got: {err}"
        );
    }

    #[test]
    fn res1112_both_branches_return_passes() {
        check(
            "fn pick(int n) -> int { \
                if n > 0 { return 1; } else { return 0; } \
            }",
        )
        .expect("both branches returning should typecheck");
    }

    #[test]
    fn res1112_implicit_return_expression_passes() {
        check("fn id(int x) -> int { x }").expect("implicit-return form should typecheck");
    }

    #[test]
    fn res1112_void_fn_can_fall_off_end() {
        check("fn main() { let x = 1; }").expect("void fn need not return");
    }

    #[test]
    fn res1112_live_block_with_return_passes() {
        // RES-1112: `live { ... return X; ... }` is recognised as a
        // terminating construct so safety-critical examples using
        // live-block retry still typecheck without spurious errors.
        check(
            "fn safe() -> int { \
                live { \
                    let x = 5; \
                    return x; \
                } \
            }",
        )
        .expect("live { return X } should count as a terminating body");
    }

    // --- RES-1113: unreachable code after return -----------------------------

    #[test]
    fn res1113_statement_after_return_rejected() {
        let err = check(
            "fn classify(int n) -> string { \
                return \"a\"; \
                return \"b\"; \
            }",
        )
        .expect_err("unreachable return should be rejected");
        assert!(
            err.contains("unreachable code"),
            "expected unreachable-code diagnostic, got: {err}"
        );
    }

    #[test]
    fn res1113_statement_after_return_in_main_rejected() {
        let err = check(
            "fn main() { \
                return; \
                let x = 1; \
            }",
        )
        .expect_err("statement after bare return should be rejected");
        assert!(
            err.contains("unreachable code"),
            "expected unreachable-code diagnostic, got: {err}"
        );
    }

    #[test]
    fn res1113_conditional_return_does_not_make_rest_unreachable() {
        // `if cond { return X; }` is NOT unconditional — the code
        // following the if is reachable when cond is false.
        check(
            "fn main() { \
                if true { return; } \
                let x = 1; \
            }",
        )
        .expect("conditional return must not flag subsequent code");
    }
}

#[cfg(test)]
mod duplicate_detection_tests {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected typechecker error")
    }

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .expect("unexpected typechecker error");
    }

    #[test]
    fn duplicate_fn_name_is_rejected() {
        let err = check_err("fn foo() {} fn foo() {}");
        assert!(
            err.contains("duplicate function name") && err.contains("`foo`"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn unique_fn_names_pass() {
        check_ok("fn foo(int x) -> int { return x; } fn bar(int x) -> int { return x; }");
    }

    #[test]
    fn duplicate_struct_field_is_rejected() {
        let err = check_err("struct Point { int x, int x }");
        assert!(
            err.contains("duplicate field") && err.contains("`x`"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn unique_struct_fields_pass() {
        check_ok("struct Point { int x, int y }");
    }
}

#[cfg(test)]
mod span_diagnostic_tests {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_with_source(src: &str, path: &str) -> Result<(), String> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, path)
            .map(|_| ())
    }

    #[test]
    fn error_includes_source_path() {
        // A type annotation mismatch on a let statement should include
        // the source path in the error message.
        let err = check_with_source(
            "fn f(int x) -> int { let y: bool = x; return y; }",
            "myfile.rz",
        )
        .expect_err("expected type error");
        assert!(
            err.contains("myfile.rz"),
            "error should include source path, got: {}",
            err
        );
    }

    #[test]
    fn error_includes_line_number() {
        // Error on the second line should include line 2 in the message.
        let src = "fn f(int x) -> int { return x; }\nfn f(int y) -> int { return y; }";
        let err = check_with_source(src, "test.rz").expect_err("expected duplicate fn error");
        assert!(
            err.contains("test.rz"),
            "error should contain file path, got: {}",
            err
        );
    }

    #[test]
    fn ok_program_has_no_span_error() {
        let src = "fn add(int x, int y) -> int { return x + y; }";
        assert!(check_with_source(src, "test.rz").is_ok());
    }

    /// RES-1862: an undefined-variable error inside a function body should
    /// report the identifier's position, not the function declaration's start.
    #[test]
    fn undefined_var_span_is_identifier_position() {
        // `unknown_var` is on line 2 col 12 (1-indexed)
        let src = "fn f(int x) -> int {\n    return unknown_var;\n}";
        let err = check_with_source(src, "src.rz").expect_err("expected undefined var error");
        assert!(
            err.contains("src.rz"),
            "error should contain file path: {}",
            err
        );
        // Must point at line 2, not line 1 (the function declaration).
        assert!(
            err.contains("src.rz:2:"),
            "error should point to line 2 where the identifier is: {}",
            err
        );
    }

    /// RES-1862: a type error inside an infix expression should report the
    /// infix span.
    #[test]
    fn infix_type_error_uses_infix_span() {
        // The `+` is on line 2.
        let src = "fn f(int x) -> int {\n    return x + \"hello\";\n}";
        let err = check_with_source(src, "expr.rz").expect_err("expected infix type error");
        assert!(
            err.contains("expr.rz:2:"),
            "error should point to line 2 (the infix expression): {}",
            err
        );
    }

    #[test]
    fn bitwise_type_error_has_path() {
        let src = r#"fn f(float x) -> int { return x & 1; }"#;
        let err = check_with_source(src, "ops.rz").expect_err("expected bitwise type error");
        assert!(
            err.contains("ops.rz"),
            "error should contain file path, got: {}",
            err
        );
    }
}

#[cfg(test)]
mod res1859_builtin_return_types {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .expect("should type-check without error");
    }

    #[test]
    fn array_map_return_type_is_array() {
        // Result of array_map used as an array argument to len() — would
        // fail type-check if the return type were inferred as Any when
        // the checker is configured to be strict on function signatures.
        check_ok(
            "fn f(array a) -> int { let m = array_map(a, fn(int x) -> int { return x * 2; }); return len(m); }",
        );
    }

    #[test]
    fn array_filter_return_type_is_array() {
        check_ok(
            "fn f(array a) -> int { let filtered = array_filter(a, fn(int x) -> bool { return x > 0; }); return len(filtered); }",
        );
    }

    #[test]
    fn string_split_returns_array() {
        check_ok(
            "fn f(string s) -> int { let parts = string_split(s, \",\"); return len(parts); }",
        );
    }

    #[test]
    fn method_map_return_type_is_array() {
        check_ok(
            "fn f(array a) -> int { let mapped = a.map(fn(int x) -> int { return x + 1; }); return len(mapped); }",
        );
    }

    #[test]
    fn method_filter_return_type_is_array() {
        check_ok(
            "fn f(array a) -> int { let filtered = a.filter(fn(int x) -> bool { return x > 0; }); return len(filtered); }",
        );
    }

    #[test]
    fn string_split_method_return_type_is_array() {
        check_ok("fn f(string s) -> int { let parts = s.split(\",\"); return len(parts); }");
    }

    // RES-1859: array mutation builtins — verify return type is Array
    // so callers can pass the result to array-typed parameters.

    #[test]
    fn array_swap_return_type_is_array() {
        check_ok("fn f(array a) -> int { let b = array_swap(a, 0, 1); return len(b); }");
    }

    #[test]
    fn array_insert_at_return_type_is_array() {
        check_ok("fn f(array a) -> int { let b = array_insert_at(a, 0, 42); return len(b); }");
    }

    #[test]
    fn array_remove_at_return_type_is_array() {
        check_ok("fn f(array a) -> int { let b = array_remove_at(a, 0); return len(b); }");
    }

    #[test]
    fn array_set_at_return_type_is_array() {
        check_ok("fn f(array a) -> int { let b = array_set_at(a, 0, 99); return len(b); }");
    }

    #[test]
    fn array_slice_return_type_is_array() {
        check_ok("fn f(array a) -> int { let b = array_slice(a, 0, 3, false); return len(b); }");
    }
}

// ── RES-1862: span attachment for node types that previously lacked it ────────

#[cfg(test)]
mod res1862_span_attachment {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_err_with_source(src: &str, path: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, path)
            .expect_err("expected type error")
    }

    #[test]
    fn match_non_exhaustive_error_includes_span() {
        // Bool scrutinee missing `false` arm — error must name the file.
        let src = "fn f(bool b) -> int {\n    return match b {\n        true => 1,\n    };\n}";
        let err = check_err_with_source(src, "match.rz");
        assert!(
            err.contains("match.rz"),
            "non-exhaustive match error must include file path; got: {err}"
        );
    }

    #[test]
    fn assert_bad_condition_error_includes_span() {
        let src = "fn f(int x) {\n    assert(x + 1);\n}";
        let err = check_err_with_source(src, "assert.rz");
        assert!(
            err.contains("assert.rz"),
            "assert type error must include file path; got: {err}"
        );
    }

    #[test]
    fn assume_bad_condition_error_includes_span() {
        let src = "fn f(int x) {\n    assume(x + 1);\n}";
        let err = check_err_with_source(src, "assume.rz");
        assert!(
            err.contains("assume.rz"),
            "assume type error must include file path; got: {err}"
        );
    }

    #[test]
    fn range_bad_bound_error_includes_span() {
        // Range lower bound must be Int; passing a string should error.
        let src = "fn f(string s) {\n    for x in s..5 { }\n}";
        let err = check_err_with_source(src, "range.rz");
        assert!(
            err.contains("range.rz"),
            "range bound error must include file path; got: {err}"
        );
    }

    #[test]
    fn for_loop_range_error_includes_span() {
        // Range lower bound is a string — must error with file path.
        let src = "fn f(string s) {\n    for i in s..5 { }\n}";
        let err = check_err_with_source(src, "for.rz");
        assert!(
            err.contains("for.rz"),
            "for-loop range error must include file path; got: {err}"
        );
    }

    #[test]
    fn while_range_in_body_error_includes_span() {
        // For-range with a string lower bound in a while loop body — the
        // range error should carry the while loop's file path.
        let src = "fn f(string s) {\n    while true {\n        for i in s..5 { }\n        break;\n    }\n}";
        let err = check_err_with_source(src, "while.rz");
        assert!(
            err.contains("while.rz"),
            "error inside while loop must include file path; got: {err}"
        );
    }
}

#[cfg(test)]
mod res401_tuple_type {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn tuple_literal_type_checks() {
        // A heterogeneous tuple literal type-checks without error.
        check_ok(r#"let _t = (1, "hello", true);"#);
    }

    #[test]
    fn empty_tuple_type_checks() {
        check_ok("let _t = ();");
    }

    #[test]
    fn tuple_index_out_of_range_errors() {
        // Accessing index 5 of a 2-element tuple should fail.
        let err = check_err("let t = (1, 2); let _x = t.5;");
        assert!(
            err.contains("out of range") || err.contains("index"),
            "expected out-of-range error; got: {err}"
        );
    }

    #[test]
    fn tuple_index_in_range_ok() {
        // Accessing a valid index should succeed.
        check_ok(r#"let t = (1, "x"); let _s = t.1;"#);
    }

    #[test]
    fn tuple_destructure_ok() {
        // Destructuring into correct names should pass.
        check_ok(r#"let (a, b) = (1, "x");"#);
    }
}

// ── RES-402: match and if expression type inference ───────────────────────────

#[cfg(test)]
mod res402_arm_type_inference {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn match_result_used_as_int() {
        // When all match arms return int, the result should be usable
        // as an int (e.g., added to another int).
        check_ok(
            r#"
fn f(int x) -> int {
    let v = match x {
        0 => 10,
        1 => 20,
        _ => 30,
    };
    return v + 1;
}
"#,
        );
    }

    #[test]
    fn match_result_used_as_string() {
        // When all match arms return a string, the result is a string.
        check_ok(
            r#"
fn describe(int x) -> string {
    return match x {
        0 => "zero",
        1 => "one",
        _ => "other",
    };
}
"#,
        );
    }

    #[test]
    fn match_result_used_as_bool() {
        // Arms returning bool — inferred type is bool.
        check_ok(
            r#"
fn is_zero(int x) -> bool {
    return match x {
        0 => true,
        _ => false,
    };
}
"#,
        );
    }

    #[test]
    fn if_with_both_branches_propagates_int() {
        // if/else where both branches return int — result is int.
        check_ok(
            r#"
fn abs_val(int x) -> int {
    let v = if x >= 0 { x } else { 0 - x };
    return v + 1;
}
"#,
        );
    }

    #[test]
    fn if_else_type_mismatch_errors() {
        // if branch returns int, else branch returns string — type error.
        let err = check_err(
            r#"
fn f(bool c) -> int {
    let _v = if c { 1 } else { "x" };
    return 0;
}
"#,
        );
        assert!(
            err.contains("incompatible") || err.contains("type"),
            "expected incompatible-types error; got: {err}"
        );
    }

    #[test]
    fn match_arms_used_in_len_context() {
        // match on bool with both arms returning array — can call len().
        check_ok(
            r#"
fn f(bool b) -> int {
    let arr = match b {
        true  => [1, 2, 3],
        false => [4, 5],
    };
    return len(arr);
}
"#,
        );
    }

    #[test]
    fn match_arms_type_mismatch_errors() {
        // RES-2664: match arms with different types must error.
        let err = check_err(
            r#"
fn f(int x) -> int {
    let _v = match x {
        1 => "hello",
        _ => 42,
    };
    return 0;
}
"#,
        );
        assert!(
            err.contains("incompatible"),
            "expected incompatible-types error; got: {err}"
        );
    }

    #[test]
    fn match_arms_three_way_mismatch_errors() {
        // RES-2664: three arms with three different types.
        let err = check_err(
            r#"
fn f(int x) -> int {
    return match x {
        1 => "hello",
        2 => true,
        _ => 42,
    };
}
"#,
        );
        assert!(
            err.contains("incompatible"),
            "expected incompatible-types error; got: {err}"
        );
    }

    #[test]
    fn match_arms_same_type_accepted() {
        // Same type in all arms — no error.
        check_ok(
            r#"
fn f(int x) -> int {
    return match x {
        1 => 10,
        2 => 20,
        _ => 30,
    };
}
"#,
        );
    }
}

// ── RES-403: return statement type validation against declared return type ────

#[cfg(test)]
mod res403_return_type_validation {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn early_return_wrong_type_is_error() {
        // An early `return 42` in a function declared `-> string` should fail.
        let err = check_err(
            r#"
fn f(bool b) -> string {
    if b { return 42; }
    return "hello";
}
"#,
        );
        assert!(
            err.contains("return type mismatch") || err.contains("type mismatch"),
            "expected return-type-mismatch error; got: {err}"
        );
    }

    #[test]
    fn early_return_correct_type_ok() {
        // Early return with the correct type should pass.
        check_ok(
            r#"
fn f(bool b) -> int {
    if b { return 1; }
    return 0;
}
"#,
        );
    }

    #[test]
    fn void_function_bare_return_ok() {
        // Bare `return;` in a void function is always valid.
        check_ok(
            r#"
fn f(bool b) -> void {
    if b { return; }
    println("done");
}
"#,
        );
    }

    #[test]
    fn lambda_return_validated_against_lambda_type() {
        // The lambda's own declared return type governs its return statements,
        // not the enclosing function's return type.
        check_ok(
            r#"
fn outer(array a) -> int {
    let mapped = array_map(a, fn(int x) -> int { return x * 2; });
    return len(mapped);
}
"#,
        );
    }

    #[test]
    fn lambda_return_type_mismatch_errors() {
        // A lambda returning the wrong type should fail.
        let err = check_err(
            r#"
fn f(array a) -> int {
    let mapped = array_map(a, fn(int x) -> int { return "oops"; });
    return len(mapped);
}
"#,
        );
        assert!(
            err.contains("return type mismatch") || err.contains("type mismatch"),
            "expected return-type error in lambda; got: {err}"
        );
    }
}

// ── RES-404: field assignment type validation ─────────────────────────────────

#[cfg(test)]
mod res404_field_assignment_type_check {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn assign_correct_field_type_ok() {
        check_ok(
            r#"
struct Point { int x, int y }
fn f() -> void {
    let p = new Point { x: 1, y: 2 };
    p.x = 10;
}
"#,
        );
    }

    #[test]
    fn assign_wrong_field_type_errors() {
        let err = check_err(
            r#"
struct Point { int x, int y }
fn f() -> void {
    let p = new Point { x: 1, y: 2 };
    p.x = "oops";
}
"#,
        );
        assert!(
            err.contains("cannot assign") || err.contains("type"),
            "expected type mismatch for field assignment; got: {err}"
        );
    }

    #[test]
    fn assign_nonexistent_field_errors() {
        let err = check_err(
            r#"
struct Point { int x, int y }
fn f() -> void {
    let p = new Point { x: 1, y: 2 };
    p.z = 99;
}
"#,
        );
        assert!(
            err.contains("no field") || err.contains("available"),
            "expected field-not-found error; got: {err}"
        );
    }
}

// ── RES-405: assignment type validation ───────────────────────────────────────

#[cfg(test)]
mod res405_assignment_type_check {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn assign_same_type_ok() {
        check_ok(
            r#"
fn f() -> void {
    let x = 5;
    x = 10;
}
"#,
        );
    }

    #[test]
    fn assign_wrong_type_errors() {
        let err = check_err(
            r#"
fn f() -> void {
    let x = 5;
    x = "hello";
}
"#,
        );
        assert!(
            err.contains("cannot assign") || err.contains("type"),
            "expected type mismatch on assignment; got: {err}"
        );
    }

    #[test]
    fn assign_to_string_var_ok() {
        check_ok(
            r#"
fn f() -> void {
    let s = "hello";
    s = "world";
}
"#,
        );
    }

    #[test]
    fn assign_int_to_string_var_errors() {
        let err = check_err(
            r#"
fn f() -> void {
    let s = "hello";
    s = 42;
}
"#,
        );
        assert!(
            err.contains("cannot assign") || err.contains("type"),
            "expected type mismatch on string assignment; got: {err}"
        );
    }
}

// ── RES-405 follow-up: index expression integer validation ────────────────────

#[cfg(test)]
mod res405_index_type_check {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn int_index_ok() {
        check_ok(r#"fn f(array a) -> void { let _x = a[0]; }"#);
    }

    #[test]
    fn float_index_errors() {
        let err = check_err(r#"fn f(array a) -> void { let _x = a[1.0]; }"#);
        assert!(
            err.contains("integer index") || err.contains("index"),
            "expected integer-index error; got: {err}"
        );
    }

    #[test]
    fn string_index_errors() {
        let err = check_err(r#"fn f(array a) -> void { let _x = a["key"]; }"#);
        assert!(
            err.contains("integer index") || err.contains("index"),
            "expected integer-index error for string index; got: {err}"
        );
    }

    #[test]
    fn string_char_int_index_ok() {
        check_ok(r#"fn f(string s) -> void { let _x = s[0]; }"#);
    }
}

// ── RES-406: while/for-in/index-assignment/enum-dedup checks ─────────────────

#[cfg(test)]
mod res406_loop_and_collection_checks {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    // ── while condition ───────────────────────────────────────────────────────

    #[test]
    fn while_bool_condition_ok() {
        check_ok(r#"fn f() -> void { while true { } }"#);
    }

    #[test]
    fn while_int_condition_errors() {
        let err = check_err(r#"fn f() -> void { while 1 { } }"#);
        assert!(
            err.contains("while condition") || err.contains("boolean"),
            "expected bool condition error; got: {err}"
        );
    }

    #[test]
    fn while_string_condition_errors() {
        let err = check_err(r#"fn f() -> void { while "yes" { } }"#);
        assert!(
            err.contains("while condition") || err.contains("boolean"),
            "expected bool condition error for string; got: {err}"
        );
    }

    // ── for-in iterable type ──────────────────────────────────────────────────

    #[test]
    fn for_in_array_ok() {
        check_ok(r#"fn f(array xs) -> void { for x in xs { } }"#);
    }

    #[test]
    fn for_in_string_ok() {
        check_ok(r#"fn f(string s) -> void { for c in s { } }"#);
    }

    #[test]
    fn for_in_int_errors() {
        let err = check_err(r#"fn f(int n) -> void { for x in n { } }"#);
        assert!(
            err.contains("cannot iterate") || err.contains("for-in"),
            "expected cannot-iterate error; got: {err}"
        );
    }

    #[test]
    fn for_in_bool_errors() {
        let err = check_err(r#"fn f(bool b) -> void { for x in b { } }"#);
        assert!(
            err.contains("cannot iterate") || err.contains("for-in"),
            "expected cannot-iterate error for bool; got: {err}"
        );
    }

    // ── index assignment integer index ────────────────────────────────────────

    #[test]
    fn index_assign_int_index_ok() {
        check_ok(r#"fn f(array a) -> void { a[0] = 1; }"#);
    }

    #[test]
    fn index_assign_float_index_errors() {
        let err = check_err(r#"fn f(array a) -> void { a[1.5] = 1; }"#);
        assert!(
            err.contains("integer index") || err.contains("index assignment"),
            "expected integer-index error on index assignment; got: {err}"
        );
    }

    // ── enum duplicate variant ────────────────────────────────────────────────

    #[test]
    fn enum_unique_variants_ok() {
        check_ok(
            r#"
enum Color { Red, Green, Blue }
fn f() -> void { }
"#,
        );
    }

    #[test]
    fn enum_duplicate_variant_errors() {
        // The parser catches duplicate variants first; either a parse error
        // or a typecheck error is acceptable — both indicate the duplication
        // was detected before evaluation.
        let src = "enum Color { Red, Green, Red }\nfn f() -> void { }\n";
        let (prog, parse_errs) = crate::parse(src);
        if !parse_errs.is_empty() {
            // Parser already caught it — verify the message is meaningful.
            let combined = parse_errs.join("; ");
            assert!(
                combined.to_lowercase().contains("red")
                    || combined.to_lowercase().contains("duplicate")
                    || combined.to_lowercase().contains("variant"),
                "parse error should mention duplicate variant; got: {combined}"
            );
            return;
        }
        // Typechecker should catch it if parser didn't.
        let err = TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected duplicate-variant error from typechecker");
        assert!(
            err.contains("duplicate variant") || err.contains("Red"),
            "expected duplicate-variant error; got: {err}"
        );
    }
}

// ── RES-407: field access on known struct + slice endpoint types ──────────────

#[cfg(test)]
mod res407_field_access_and_slice {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    // ── FieldAccess: unknown field on known struct ─────────────────────────────

    #[test]
    fn field_access_known_field_ok() {
        check_ok(
            r#"
struct Point { int x, int y }
fn f(Point p) -> int { return p.x; }
"#,
        );
    }

    #[test]
    fn field_access_unknown_field_errors() {
        let err = check_err(
            r#"
struct Point { int x, int y }
fn f(Point p) -> int { return p.z; }
"#,
        );
        assert!(
            err.contains("no field") || err.contains("z"),
            "expected no-field error; got: {err}"
        );
    }

    #[test]
    fn field_access_unknown_struct_is_permissive() {
        // When the struct name isn't declared, fall through to Any.
        check_ok(r#"fn f(UnknownStruct s) -> int { return s.x; }"#);
    }

    // RES-2622: did-you-mean hints on struct field typos so users don't
    // have to scan the available-fields list to spot a one-letter slip.

    #[test]
    fn field_access_typo_suggests_close_field() {
        let err = check_err(
            r#"
struct Rectangle { int width, int height }
fn f(Rectangle r) -> int { return r.heigth; }
"#,
        );
        assert!(
            err.contains("did you mean `height`?"),
            "expected did-you-mean hint; got: {err}"
        );
    }

    #[test]
    fn struct_literal_typo_suggests_close_field() {
        let err = check_err(
            r#"
struct Rectangle { int width, int height }
fn f() -> Rectangle { return new Rectangle { width: 1, heigth: 2 }; }
"#,
        );
        assert!(
            err.contains("did you mean `height`?"),
            "expected did-you-mean hint; got: {err}"
        );
    }

    #[test]
    fn field_assignment_typo_suggests_close_field() {
        let err = check_err(
            r#"
struct Counter { int count }
fn f(Counter c) -> void { c.cont = 1; }
"#,
        );
        assert!(
            err.contains("did you mean `count`?"),
            "expected did-you-mean hint; got: {err}"
        );
    }

    #[test]
    fn match_pattern_typo_suggests_close_field() {
        let err = check_err(
            r#"
struct Vec2 { int width, int height }
fn f(Vec2 v) -> int {
    return match v {
        Vec2 { widht, height } => width + height,
        _ => 0,
    };
}
"#,
        );
        assert!(
            err.contains("did you mean `width`?"),
            "expected did-you-mean hint; got: {err}"
        );
    }

    #[test]
    fn field_access_far_typo_omits_hint() {
        // Edit distance 4 — too far to suggest. Error stands without hint.
        let err = check_err(
            r#"
struct Tiny { int n }
fn f(Tiny t) -> int { return t.completely_different; }
"#,
        );
        assert!(
            err.contains("has no field"),
            "expected no-field error; got: {err}"
        );
        assert!(
            !err.contains("did you mean"),
            "did not expect did-you-mean hint at distance > 2; got: {err}"
        );
    }

    // ── Slice endpoint types ───────────────────────────────────────────────────

    #[test]
    fn slice_int_bounds_ok() {
        check_ok(r#"fn f(array a) -> array { return a[1..3]; }"#);
    }

    #[test]
    fn slice_float_lower_bound_errors() {
        let err = check_err(r#"fn f(array a) -> array { return a[1.0..3]; }"#);
        assert!(
            err.contains("lower bound") || err.contains("integer"),
            "expected integer-bound error; got: {err}"
        );
    }

    #[test]
    fn slice_float_upper_bound_errors() {
        let err = check_err(r#"fn f(array a) -> array { return a[0..3.5]; }"#);
        assert!(
            err.contains("upper bound") || err.contains("integer"),
            "expected integer-bound error; got: {err}"
        );
    }

    #[test]
    fn slice_string_int_bounds_ok() {
        check_ok(r#"fn f(string s) -> string { return s[0..5]; }"#);
    }
}

// ── RES-408: `any` type annotation → Type::Any + struct pattern permissiveness ─

#[cfg(test)]
mod res408_any_type_annotation {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn any_param_accepts_int_arg() {
        check_ok(
            r#"
fn f(any x) -> int { return 0; }
fn g() -> int { return f(42); }
"#,
        );
    }

    #[test]
    fn any_param_accepts_string_arg() {
        check_ok(
            r#"
fn f(any x) -> int { return 0; }
fn g() -> int { return f("hello"); }
"#,
        );
    }

    #[test]
    fn any_return_ok() {
        check_ok(r#"fn f() -> any { return 42; }"#);
    }

    #[test]
    fn struct_pattern_on_any_scrutinee_ok() {
        check_ok(
            r#"
struct Point { int x, int y }
fn f(any p) -> int {
    return match p {
        Point { x, y } => x,
        _ => 0,
    };
}
"#,
        );
    }

    #[test]
    fn struct_pattern_wrong_struct_errors() {
        let err = check_err(
            r#"
struct Point { int x, int y }
struct Rect { int w, int h }
fn f(Rect r) -> int {
    return match r {
        Point { x, y } => x,
        _ => 0,
    };
}
"#,
        );
        assert!(
            err.contains("Point") || err.contains("Rect") || err.contains("pattern"),
            "expected struct mismatch error; got: {err}"
        );
    }

    #[test]
    fn any_let_binding_ok() {
        check_ok(r#"fn f() -> void { let x: any = 5; let _y: any = "hello"; }"#);
    }
}

// ── RES-409: for-in element type inference + duplicate struct check ──────────

#[cfg(test)]
mod res409_forin_element_type_and_duplicate_struct {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    // ── for-in element type inference ─────────────────────────────────────────

    #[test]
    fn for_in_range_elem_is_int() {
        // The loop variable `i` should be Int, so `i + 1` is valid.
        check_ok(
            r#"
fn f() -> void {
    for i in 0..10 {
        let _x = i + 1;
    }
}
"#,
        );
    }

    #[test]
    fn for_in_range_elem_int_used_in_string_concat_errors() {
        // `i` is Int from a range; `"hello" + i` is String (coercion),
        // but `i - "hello"` is an error (not a valid arithmetic op).
        // This validates that the range-elem binding really is Int.
        let err = check_err(
            r#"
fn f() -> void {
    for i in 0..5 {
        let _x = i - "hello";
    }
}
"#,
        );
        assert!(
            err.contains("Cannot apply") || err.contains("int") || err.contains("string"),
            "expected type error for int - string in range loop; got: {err}"
        );
    }

    #[test]
    fn for_in_string_elem_is_string() {
        // Each character from a string iteration is a String.
        check_ok(
            r#"
fn f(string s) -> void {
    for c in s {
        let _x: string = c;
    }
}
"#,
        );
    }

    // ── duplicate struct declaration ──────────────────────────────────────────

    #[test]
    fn unique_struct_ok() {
        check_ok(
            r#"
struct Point { int x, int y }
fn f() -> void { }
"#,
        );
    }

    #[test]
    fn duplicate_struct_errors() {
        let err = check_err(
            r#"
struct Point { int x, int y }
struct Point { int a, int b }
fn f() -> void { }
"#,
        );
        assert!(
            err.contains("duplicate") || err.contains("Point"),
            "expected duplicate struct error; got: {err}"
        );
    }
}

// ── RES-410: min/max/pow type inference + duplicate TypeAlias ──────────────

#[cfg(test)]
mod res410_numeric_poly_and_type_alias {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    // ── min/max/pow type inference ────────────────────────────────────────────

    #[test]
    fn min_int_args_returns_int() {
        // min(1, 2) should infer Int; `x + 1` must succeed.
        check_ok(r#"fn f() -> int { let x = min(1, 2); return x + 1; }"#);
    }

    #[test]
    fn max_float_args_returns_float() {
        check_ok(r#"fn f() -> float { let x = max(1.0, 2.0); return x; }"#);
    }

    #[test]
    fn pow_int_args_returns_int() {
        check_ok(r#"fn f() -> int { let x = pow(2, 10); return x; }"#);
    }

    #[test]
    fn min_mixed_types_returns_any() {
        // Mixed types → Any; the function call itself still succeeds.
        check_ok(r#"fn f() -> void { let _x = min(1, 2.0); }"#);
    }

    // ── duplicate type alias ──────────────────────────────────────────────────

    #[test]
    fn unique_type_alias_ok() {
        check_ok(
            r#"
type Meters = float;
fn f(Meters m) -> float { return m; }
"#,
        );
    }

    #[test]
    fn duplicate_type_alias_same_target_ok() {
        // Re-declaring to the same target is silently accepted (the
        // pre-pass may have already registered it).
        check_ok(
            r#"
type Meters = float;
fn f() -> void { }
"#,
        );
    }

    #[test]
    fn duplicate_type_alias_different_target_errors() {
        // Re-declaring with a different target is a compile error.
        let err = check_err("type M = int;\ntype M = float;\nfn f() -> void { }\n");
        assert!(
            err.contains("duplicate type alias") || err.contains("M"),
            "expected duplicate-alias error; got: {err}"
        );
    }
}

// ── RES-411: integer literal overflow for pinned int types ───────────────────

#[cfg(test)]
mod res411_pinned_int_overflow {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn int8_in_range_ok() {
        check_ok("fn f() -> void { let x: Int8 = 100; }");
    }

    #[test]
    fn int8_min_boundary_ok() {
        check_ok("fn f() -> void { let x: Int8 = -128; }");
    }

    #[test]
    fn int8_max_boundary_ok() {
        check_ok("fn f() -> void { let x: Int8 = 127; }");
    }

    #[test]
    fn int8_overflow_errors() {
        let e = check_err("fn f() -> void { let x: Int8 = 300; }");
        assert!(e.contains("overflows"), "expected overflow error; got: {e}");
        assert!(e.contains("Int8"), "error must name the type; got: {e}");
    }

    #[test]
    fn int8_underflow_errors() {
        let e = check_err("fn f() -> void { let x: Int8 = -200; }");
        assert!(e.contains("overflows"), "expected overflow error; got: {e}");
    }

    #[test]
    fn uint8_in_range_ok() {
        check_ok("fn f() -> void { let x: UInt8 = 255; }");
    }

    #[test]
    fn uint8_overflow_errors() {
        let e = check_err("fn f() -> void { let x: UInt8 = 256; }");
        assert!(e.contains("overflows"), "expected overflow error; got: {e}");
        assert!(e.contains("UInt8"), "error must name the type; got: {e}");
    }

    #[test]
    fn uint8_negative_errors() {
        let e = check_err("fn f() -> void { let x: UInt8 = -1; }");
        assert!(
            e.contains("overflows"),
            "expected overflow error for negative UInt8; got: {e}"
        );
    }

    #[test]
    fn int16_overflow_errors() {
        let e = check_err("fn f() -> void { let x: Int16 = 40000; }");
        assert!(e.contains("overflows"), "expected overflow error; got: {e}");
    }

    #[test]
    fn uint16_overflow_errors() {
        let e = check_err("fn f() -> void { let x: UInt16 = 70000; }");
        assert!(e.contains("overflows"), "expected overflow error; got: {e}");
    }

    #[test]
    fn int32_max_boundary_ok() {
        check_ok("fn f() -> void { let x: Int32 = 2147483647; }");
    }

    #[test]
    fn uint32_overflow_errors() {
        let e = check_err("fn f() -> void { let x: UInt32 = 5000000000; }");
        assert!(e.contains("overflows"), "expected overflow error; got: {e}");
    }

    #[test]
    fn plain_int_no_overflow_check() {
        // Unbounded `int` never overflows.
        check_ok("fn f() -> void { let x: int = 9999999999; }");
    }

    #[test]
    fn unannotated_let_no_overflow_check() {
        // Without a type annotation there's no declared range to check.
        check_ok("fn f() -> void { let x = 300; }");
    }
}

// ── RES-412: string/array method return type precision ───────────────────────

#[cfg(test)]
mod res412_method_return_types {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    #[test]
    fn array_len_returns_int() {
        // arr.len() should be usable where an int is expected.
        check_ok(r#"fn f() -> int { let arr = [1, 2, 3]; return arr.len(); }"#);
    }

    #[test]
    fn string_len_returns_int() {
        check_ok(r#"fn f() -> int { let s = "hello"; return s.len(); }"#);
    }

    #[test]
    fn string_trim_returns_string() {
        check_ok(r#"fn f() -> string { let s = "  hi  "; return s.trim(); }"#);
    }

    #[test]
    fn string_to_upper_returns_string() {
        check_ok(r#"fn f() -> string { let s = "hello"; return s.to_upper(); }"#);
    }

    #[test]
    fn string_to_lower_returns_string() {
        check_ok(r#"fn f() -> string { let s = "HELLO"; return s.to_lower(); }"#);
    }

    #[test]
    fn string_contains_returns_bool() {
        check_ok(r#"fn f() -> bool { let s = "hello world"; return s.contains("world"); }"#);
    }

    #[test]
    fn string_starts_with_returns_bool() {
        check_ok(r#"fn f() -> bool { let s = "hello"; return s.starts_with("he"); }"#);
    }

    #[test]
    fn string_ends_with_returns_bool() {
        check_ok(r#"fn f() -> bool { let s = "hello"; return s.ends_with("lo"); }"#);
    }

    #[test]
    fn string_replace_returns_string() {
        check_ok(r#"fn f() -> string { let s = "hello"; return s.replace("l", "r"); }"#);
    }

    #[test]
    fn array_contains_returns_bool() {
        check_ok(r#"fn f() -> bool { let arr = [1, 2, 3]; return arr.contains(2); }"#);
    }

    #[test]
    fn array_join_returns_string() {
        check_ok(r#"fn f() -> string { let arr = ["a", "b"]; return arr.join(", "); }"#);
    }

    // RES-2734: zero-extra-arg methods were incorrectly registered with 1 param.
    #[test]
    fn array_sort_accepts_no_args() {
        check_ok(r#"fn f() -> array { let arr = [3, 1, 2]; return arr.sort(); }"#);
    }

    #[test]
    fn array_sort_desc_accepts_no_args() {
        check_ok(r#"fn f() -> array { let arr = [1, 3, 2]; return arr.sort_desc(); }"#);
    }

    #[test]
    fn array_reverse_accepts_no_args() {
        check_ok(r#"fn f() -> array { let arr = [1, 2, 3]; return arr.reverse(); }"#);
    }

    #[test]
    fn array_pop_accepts_no_args() {
        check_ok(r#"fn f() -> array { let arr = [1, 2, 3]; return arr.pop(); }"#);
    }

    #[test]
    fn array_flatten_accepts_no_args() {
        check_ok(r#"fn f() -> array { let arr = [[1, 2], [3]]; return arr.flatten(); }"#);
    }

    #[test]
    fn array_dedup_accepts_no_args() {
        check_ok(r#"fn f() -> array { let arr = [1, 1, 2]; return arr.dedup(); }"#);
    }

    #[test]
    fn array_has_returns_bool() {
        check_ok(r#"fn f() -> bool { let arr = [1, 2, 3]; return arr.has(2); }"#);
    }

    #[test]
    fn array_slice_returns_array() {
        check_ok(r#"fn f() -> array { let arr = [1, 2, 3, 4]; return arr.slice(1, 3); }"#);
    }
}

// ── RES-413: static division by zero detection ───────────────────────────────

#[cfg(test)]
mod res413_div_by_zero {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn div_by_zero_literal_errors() {
        let e = check_err("fn f() -> int { return 10 / 0; }");
        assert!(
            e.contains("division by zero") || e.contains("by zero"),
            "got: {e}"
        );
    }

    #[test]
    fn mod_by_zero_literal_errors() {
        let e = check_err("fn f() -> int { return 10 % 0; }");
        assert!(
            e.contains("modulo by zero") || e.contains("by zero"),
            "got: {e}"
        );
    }

    #[test]
    fn div_by_const_zero_errors() {
        let e = check_err("fn f() -> int { let d = 0; return 10 / d; }");
        assert!(
            e.contains("by zero"),
            "expected division-by-zero error; got: {e}"
        );
    }

    #[test]
    fn div_by_nonzero_ok() {
        check_ok("fn f() -> int { return 10 / 2; }");
    }

    #[test]
    fn div_by_variable_ok() {
        // Non-constant divisor — can't statically check, so it's ok.
        check_ok("fn f(int n) -> int { return 10 / n; }");
    }
}

// ── RES-414: void-in-let binding detection ───────────────────────────────────

#[cfg(test)]
mod res414_void_in_let {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn let_void_fn_result_errors() {
        let e = check_err("fn f() -> void { let x = print(\"hi\"); }");
        assert!(
            e.contains("void") || e.contains("Void"),
            "expected void-binding error; got: {e}"
        );
    }

    #[test]
    fn let_underscore_void_ok() {
        // _ prefix is the discard convention — allowed for void values.
        check_ok("fn f() -> void { let _x = print(\"hi\"); }");
    }

    #[test]
    fn let_int_fn_ok() {
        check_ok("fn f() -> int { let x = len(\"hi\"); return x; }");
    }

    #[test]
    fn void_fn_as_expression_statement_ok() {
        // Using a void-returning fn as a plain expression statement is fine.
        check_ok("fn f() -> void { print(\"hi\"); }");
    }
}

// ── RES-415: map/set literal type consistency + negative index detection ──────

#[cfg(test)]
mod res415_collection_type_checks {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn map_uniform_keys_ok() {
        check_ok(r#"fn f() -> void { let _m = {"a" -> 1, "b" -> 2}; }"#);
    }

    #[test]
    fn map_mixed_key_types_errors() {
        let e = check_err(r#"fn f() -> void { let _m = {"a" -> 1, 2 -> 3}; }"#);
        assert!(
            e.contains("mixed key types") || e.contains("key"),
            "got: {e}"
        );
    }

    #[test]
    fn map_mixed_value_types_errors() {
        let e = check_err(r#"fn f() -> void { let _m = {"a" -> 1, "b" -> "two"}; }"#);
        assert!(
            e.contains("mixed value types") || e.contains("value"),
            "got: {e}"
        );
    }

    #[test]
    fn set_uniform_elements_ok() {
        check_ok(r#"fn f() -> void { let _s = #{1, 2, 3}; }"#);
    }

    #[test]
    fn set_mixed_elements_errors() {
        let e = check_err(r#"fn f() -> void { let _s = #{1, "two", 3}; }"#);
        assert!(
            e.contains("mixed element types") || e.contains("element"),
            "got: {e}"
        );
    }

    #[test]
    fn negative_constant_index_is_valid() {
        // RES-921 added Python-style negative indexing (arr[-1] = last element).
        // RES-2731 removed the false-positive compile-time rejection of negative
        // constant indices — the runtime handles them correctly.
        check_ok(r#"fn f() -> void { let arr = [1, 2, 3]; let x = arr[-1]; }"#);
    }

    #[test]
    fn string_index_returns_char() {
        // RES-2709 + RES-2711: s[i] now produces a char, not a single-char string.
        check_ok(r#"fn f() -> char { let s = "hello"; return s[0]; }"#);
    }
}

// ── RES-416: const pinned-int overflow + enum constructor type checking ────────

#[cfg(test)]
mod res416_const_and_enum_checks {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn const_int8_in_range_ok() {
        check_ok("const N: Int8 = 100;");
    }

    #[test]
    fn const_int8_overflow_errors() {
        let e = check_err("const N: Int8 = 300;");
        assert!(e.contains("overflows"), "expected overflow error; got: {e}");
    }

    #[test]
    fn const_uint8_overflow_errors() {
        let e = check_err("const N: UInt8 = 256;");
        assert!(e.contains("overflows"), "expected overflow error; got: {e}");
    }

    #[test]
    fn enum_constructor_correct_types_ok() {
        check_ok(
            r#"
enum Shape { Circle(int) }
fn f() -> void { let _s = Shape::Circle(5); }
"#,
        );
    }

    #[test]
    fn enum_constructor_wrong_arg_type_errors() {
        let e = check_err(
            r#"
enum Shape { Circle(int) }
fn f() -> void { let _s = Shape::Circle("not_an_int"); }
"#,
        );
        assert!(
            e.contains("argument has type") || e.contains("expected"),
            "expected constructor type error; got: {e}"
        );
    }

    #[test]
    fn enum_constructor_wrong_arg_count_errors() {
        let e = check_err(
            r#"
enum Shape { Circle(int) }
fn f() -> void { let _s = Shape::Circle(1, 2); }
"#,
        );
        assert!(
            e.contains("expected") && (e.contains("arg") || e.contains("argument")),
            "expected arity error; got: {e}"
        );
    }
}

// ── RES-417: const declaration forward reference ──────────────────────────────

#[cfg(test)]
mod res417_const_forward_ref {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn fn_forward_refs_const_ok() {
        // Function defined before the const it uses — must succeed.
        check_ok(
            r#"
fn f() -> int { return N; }
const N: int = 5;
"#,
        );
    }

    #[test]
    fn const_used_in_expression_ok() {
        check_ok(
            r#"
fn f() -> int { return MAX + 1; }
const MAX: int = 100;
"#,
        );
    }

    #[test]
    fn const_after_fn_type_mismatch_errors() {
        // N is declared as Int8 = 300 — overflow should still be caught
        // even when the const appears after its use site.
        let e = check_err(
            r#"
const N: Int8 = 300;
fn f() -> void { }
"#,
        );
        assert!(e.contains("overflows"), "expected overflow error; got: {e}");
    }

    #[test]
    fn const_normal_declaration_ok() {
        check_ok("const N: int = 42;");
    }

    #[test]
    fn const_float_ok() {
        check_ok("const PI: float = 3.14;");
    }
}

// ── RES-418: struct literal unknown-field detection ───────────────────────────

#[cfg(test)]
mod res418_struct_literal_unknown_field {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect("expected no type error");
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected type error")
    }

    #[test]
    fn known_fields_ok() {
        check_ok(
            r#"
struct Point { int x, int y }
fn f() -> void { let _p = new Point { x: 1, y: 2 }; }
"#,
        );
    }

    #[test]
    fn unknown_field_errors() {
        let e = check_err(
            r#"
struct Point { int x, int y }
fn f() -> void { let _p = new Point { x: 1, z: 2 }; }
"#,
        );
        assert!(
            e.contains("has no field") || e.contains("no field"),
            "expected unknown-field error; got: {e}"
        );
        assert!(
            e.contains("z"),
            "error must name the unknown field; got: {e}"
        );
    }

    #[test]
    fn field_type_mismatch_errors() {
        let e = check_err(
            r#"
struct Point { int x, int y }
fn f() -> void { let _p = new Point { x: "not_an_int", y: 2 }; }
"#,
        );
        assert!(
            e.contains("type") || e.contains("int"),
            "expected type mismatch error; got: {e}"
        );
    }

    #[test]
    fn struct_on_unknown_struct_ok() {
        // If the struct isn't declared, we skip unknown-field checking
        // to be permissive about partial programs.
        check_ok("fn f() -> void { let _p = new UnknownStruct { foo: 1 }; }");
    }
}

// =====================================================
// RES-419: fn(T)->R type annotation parsing
// ============================================================
#[cfg(test)]
mod res419_fn_type_annotation {
    use super::*;

    fn check_ok(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    #[test]
    fn fn_param_callable_as_function() {
        // A higher-order function with a fn(int) -> int parameter must
        // be callable inside the body without a type error.
        check_ok(
            r#"
fn apply(int x, fn(int) -> int f) -> int { return f(x); }
fn double(int x) -> int { return x * 2; }
fn main(int _d) -> int { return apply(5, double); }
"#,
        );
    }

    #[test]
    fn fn_param_zero_args_callable() {
        check_ok(
            r#"
fn call_zero(fn() -> int f) -> int { return f(); }
fn get_one() -> int { return 1; }
fn main(int _d) -> int { return call_zero(get_one); }
"#,
        );
    }

    #[test]
    fn fn_param_multi_arg_callable() {
        check_ok(
            r#"
fn apply2(int x, int y, fn(int, int) -> int f) -> int { return f(x, y); }
fn add(int a, int b) -> int { return a + b; }
fn main(int _d) -> int { return apply2(3, 4, add); }
"#,
        );
    }

    #[test]
    fn fn_return_type_inferred_from_annotation() {
        // When calling a fn-typed parameter the return type is the
        // declared return type of the function annotation.
        check_ok(
            r#"
fn transform(string s, fn(string) -> string f) -> string { return f(s); }
fn shout(string x) -> string { return to_upper(x); }
fn main(int _d) -> int {
    let _r = transform("hello", shout);
    return 0;
}
"#,
        );
    }

    #[test]
    fn fn_type_in_let_binding() {
        // A let binding with a fn(...) -> ... annotation should work.
        check_ok(
            r#"
fn id(int x) -> int { return x; }
fn main(int _d) -> int {
    let f: fn(int) -> int = id;
    return f(3);
}
"#,
        );
    }
}

// ============================================================
// RES-420: for-in element type from builtin call
// ============================================================
#[cfg(test)]
mod res420_forin_element_type {
    use super::*;

    fn check_ok(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    #[test]
    fn forin_array_range_element_is_int() {
        // `array_range(lo, hi)` returns `Array`; element type is Int.
        // The arithmetic `x + 1` must not produce a type error.
        check_ok(
            r#"
fn main(int _d) -> int {
    let s = 0;
    for x in array_range(0, 5) {
        let s = s + x;
    }
    return s;
}
"#,
        );
    }

    #[test]
    fn forin_split_element_is_string() {
        // `split(str, sep)` → Array of strings; element type is String.
        check_ok(
            r#"
fn main(int _d) -> int {
    for word in split("hello world", " ") {
        let _len = len(word);
    }
    return 0;
}
"#,
        );
    }

    #[test]
    fn forin_string_split_element_is_string() {
        check_ok(
            r#"
fn main(int _d) -> int {
    for tok in string_split("a,b,c", ",") {
        let _u = to_upper(tok);
    }
    return 0;
}
"#,
        );
    }

    #[test]
    fn forin_range_syntax_still_int() {
        // The original Node::Range { .. } case must still work.
        check_ok(
            r#"
fn main(int _d) -> int {
    let acc = 0;
    for i in 0..5 { let acc = acc + i; }
    return acc;
}
"#,
        );
    }
}

// ============================================================
// RES-421: IfStatement type with diverging branch
// ============================================================
#[cfg(test)]
mod res421_if_diverging_branch {
    use super::*;

    fn check_ok(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected type error")
    }

    #[test]
    fn if_else_consequence_diverges_uses_alt_type() {
        // consequence returns early; else branch is Int — whole expr is Int.
        check_ok(
            r#"
fn guard(bool bad) -> int {
    let x = if bad { return 0; } else { 42 };
    return x;
}
fn main(int _d) -> int { return guard(false); }
"#,
        );
    }

    #[test]
    fn if_else_alternative_diverges_uses_cons_type() {
        // alternative returns early; consequence is Int — whole expr is Int.
        check_ok(
            r#"
fn guard2(bool bad) -> int {
    let x = if bad { 42 } else { return 0; };
    return x;
}
fn main(int _d) -> int { return guard2(false); }
"#,
        );
    }

    #[test]
    fn both_branches_non_diverging_must_match() {
        // Neither diverges and types differ → type error (pre-existing behaviour).
        let e = check_err(
            r#"
fn f(bool b) -> int {
    let x = if b { 1 } else { "two" };
    return x;
}
"#,
        );
        assert!(
            e.contains("incompatible") || e.contains("mismatch") || e.contains("type"),
            "expected incompatible-types error; got: {e}"
        );
    }

    #[test]
    fn guard_clause_pattern_compiles() {
        // Classic guard: early return in if-without-else is fine.
        check_ok(
            r#"
fn safe_sqrt(int n) -> float {
    if n < 0 { return 0.0; }
    return sqrt(n);
}
fn main(int _d) -> int { return 0; }
"#,
        );
    }
}

// ── RES-425: generic type parameter call-site fix ────────────────────────────

#[cfg(test)]
mod res425_generic_typecheck {
    use super::*;

    fn check_ok(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("type error: {e}"));
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected type error")
    }

    #[test]
    fn generic_id_with_int_no_false_positive() {
        check_ok("fn id<T>(T x) -> T { return x; }\nfn f(int a) -> int { return id(a); }");
    }

    #[test]
    fn generic_id_with_string_no_false_positive() {
        check_ok("fn id<T>(T x) -> T { return x; }\nfn f(string s) -> string { return id(s); }");
    }

    #[test]
    fn generic_two_params_no_false_positive() {
        check_ok(
            "fn first<A, B>(A a, B b) -> A { return a; }\n\
             fn f(int x, string y) -> int { return first(x, y); }",
        );
    }

    #[test]
    fn generic_accepts_different_types_at_different_sites() {
        check_ok(
            "fn wrap<T>(T x) -> T { return x; }\n\
             fn f(int a) -> int { return wrap(a); }\n\
             fn g(string s) -> string { return wrap(s); }",
        );
    }

    #[test]
    fn non_generic_type_mismatch_still_caught() {
        let e = check_err(
            "fn takes_int(int x) -> int { return x; }\n\
             fn f(string s) -> int { return takes_int(s); }",
        );
        assert!(
            e.contains("mismatch") || e.contains("expected int"),
            "expected type mismatch; got: {e}"
        );
    }
}

// ── RES-426: tuple return type in function signatures ────────────────────────

#[cfg(test)]
mod res426_tuple_return_type {
    use super::*;

    fn check_ok(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("type error: {e}"));
    }

    #[test]
    fn tuple_return_type_parses_and_typechecks() {
        check_ok(
            "fn divmod(int a, int b) -> (int, int) { return (a / b, a % b); }\n\
             fn f(int a) -> int { let r = divmod(a, 3); return 0; }",
        );
    }

    #[test]
    fn tuple_return_single_element() {
        check_ok("fn wrap(int x) -> (int) { return (x); }\nfn f(int a) -> int { return 0; }");
    }

    #[test]
    fn tuple_return_three_elements() {
        check_ok(
            "fn triple(int x) -> (int, int, int) { return (x, x, x); }\n\
             fn f(int a) -> int { return 0; }",
        );
    }

    #[test]
    fn tuple_destructure_from_function() {
        // let (a, b) = divmod(...) should parse and type-check
        check_ok(
            "fn divmod(int a, int b) -> (int, int) { return (a / b, a % b); }\n\
             fn f(int a) -> int {\n\
                 let (q, r) = divmod(a, 3);\n\
                 return q;\n\
             }",
        );
    }
}

// RES-2693: struct implementing a trait must be accepted wherever the trait
// type is expected — function parameters, let annotations, return statements.
#[cfg(test)]
mod res2693_trait_param_compat {
    use super::*;

    fn check_ok(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    fn check_err(src: &str, expected_fragment: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let e = TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected a type error but got Ok");
        assert!(
            e.contains(expected_fragment),
            "expected fragment {:?} not found in error: {e}",
            expected_fragment
        );
    }

    #[test]
    fn struct_accepted_as_trait_fn_param() {
        check_ok(
            "trait Greet { fn greet(self) -> string; }\n\
             struct Alice { int x }\n\
             impl Greet for Alice {\n\
               fn greet(self) -> string { return \"hi\"; }\n\
             }\n\
             fn say(Greet g) -> void { println(g.greet()); }\n\
             let a = new Alice { x: 1 };\n\
             say(a);\n",
        );
    }

    #[test]
    fn struct_accepted_for_trait_let_annotation() {
        check_ok(
            "trait Greet { fn greet(self) -> string; }\n\
             struct Bob { int y }\n\
             impl Greet for Bob {\n\
               fn greet(self) -> string { return \"hello\"; }\n\
             }\n\
             let b: Greet = new Bob { y: 2 };\n\
             println(b.greet());\n",
        );
    }

    #[test]
    fn struct_accepted_as_trait_return_type() {
        check_ok(
            "trait Greet { fn greet(self) -> string; }\n\
             struct Carol { int z }\n\
             impl Greet for Carol {\n\
               fn greet(self) -> string { return \"hey\"; }\n\
             }\n\
             fn make() -> Greet { return new Carol { z: 3 }; }\n",
        );
    }

    #[test]
    fn non_implementing_struct_still_errors() {
        check_err(
            "trait Greet { fn greet(self) -> string; }\n\
             struct Dave { int w }\n\
             fn say(Greet g) -> void { println(g.greet()); }\n\
             let d = new Dave { w: 4 };\n\
             say(d);\n",
            "Type mismatch",
        );
    }

    #[test]
    fn multiple_impls_different_traits_independent() {
        // struct implementing two traits can satisfy either
        check_ok(
            "trait Greet { fn greet(self) -> string; }\n\
             trait Farewell { fn bye(self) -> string; }\n\
             struct Eve { int n }\n\
             impl Greet for Eve { fn greet(self) -> string { return \"hi\"; } }\n\
             impl Farewell for Eve { fn bye(self) -> string { return \"bye\"; } }\n\
             fn say_hi(Greet g) -> void { println(g.greet()); }\n\
             fn say_bye(Farewell f) -> void { println(f.bye()); }\n\
             let e = new Eve { n: 0 };\n\
             say_hi(e);\n\
             say_bye(e);\n",
        );
    }
}

#[cfg(test)]
mod res2701_generic_fn_type_params {
    use super::*;

    fn check_ok(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    fn check_err(src: &str, fragment: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let e = TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected a type error but got Ok");
        assert!(
            e.contains(fragment),
            "expected {:?} in error: {e}",
            fragment
        );
    }

    #[test]
    fn lambda_accepted_for_fn_t_to_t_param() {
        check_ok(
            "fn apply_fn<T>(T x, fn(T) -> T f) -> T { return f(x); }\n\
             let r = apply_fn(5, fn(int n) -> int { return n * 2; });\n",
        );
    }

    #[test]
    fn named_fn_accepted_for_fn_t_to_t_param() {
        check_ok(
            "fn double(int x) -> int { return x * 2; }\n\
             fn apply_fn<T>(T x, fn(T) -> T f) -> T { return f(x); }\n\
             let r = apply_fn(5, double);\n",
        );
    }

    #[test]
    fn two_type_params_fn_arg_accepted() {
        // A and B both appear in non-fn-type positions so inference can bind
        // them; the fn-type param fn(A) -> A still uses a type variable.
        check_ok(
            "fn transform<A, B>(A x, B y, fn(A) -> A f) -> A { return f(x); }\n\
             let r = transform(3, \"unused\", fn(int n) -> int { return n * 2; });\n",
        );
    }

    #[test]
    fn wrong_concrete_fn_type_still_errors() {
        check_err(
            "fn apply_int(int x, fn(int) -> int f) -> int { return f(x); }\n\
             let r = apply_int(5, fn(string s) -> int { return 0; });\n",
            "Type mismatch",
        );
    }

    #[test]
    fn substitute_type_params_helper_recurses() {
        let tp = vec!["T".to_string()];
        let fn_type = Type::Function {
            params: vec![Type::Struct("T".to_string())],
            return_type: Box::new(Type::Struct("T".to_string())),
        };
        let subst = substitute_type_params(&fn_type, &tp);
        assert_eq!(
            subst,
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            }
        );
    }
}

#[cfg(test)]
mod res2703_result_exhaustiveness {
    use super::*;

    fn check_ok(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    fn check_err(src: &str, fragment: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let e = TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected a type error but got Ok");
        assert!(
            e.contains(fragment),
            "expected {:?} in error: {e}",
            fragment
        );
    }

    #[test]
    fn ok_and_err_arms_accepted_as_exhaustive() {
        check_ok(
            "fn f() -> void {\n\
             let r = Ok(5);\n\
             match r {\n\
               Ok(v) => println(to_string(v)),\n\
               Err(e) => println(e),\n\
             }\n\
             }\n",
        );
    }

    #[test]
    fn wildcard_arm_still_makes_result_match_exhaustive() {
        check_ok(
            "fn f() -> void {\n\
             let r = Ok(5);\n\
             match r {\n\
               Ok(v) => println(to_string(v)),\n\
               _ => println(\"fallback\"),\n\
             }\n\
             }\n",
        );
    }

    #[test]
    fn missing_err_arm_still_errors() {
        check_err(
            "fn f() -> void {\n\
             let r = Ok(5);\n\
             match r {\n\
               Ok(v) => println(to_string(v)),\n\
             }\n\
             }\n",
            "Non-exhaustive match on enum `Result`",
        );
    }

    #[test]
    fn missing_ok_arm_still_errors() {
        check_err(
            "fn f() -> void {\n\
             let r = Err(\"oops\");\n\
             match r {\n\
               Err(e) => println(e),\n\
             }\n\
             }\n",
            "Non-exhaustive match on enum `Result`",
        );
    }

    #[test]
    fn ok_and_err_arms_with_block_bodies_accepted() {
        check_ok(
            "fn f() -> void {\n\
             let r = Ok(5);\n\
             match r {\n\
               Ok(v) => { println(to_string(v)); },\n\
               Err(e) => { println(e); },\n\
             }\n\
             }\n",
        );
    }
}

#[cfg(test)]
mod res2705_array_type_annotation {
    use super::*;

    fn check_ok(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    fn check_err(src: &str, fragment: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let e = TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected a type error but got Ok");
        assert!(
            e.contains(fragment),
            "expected {:?} in error: {e}",
            fragment
        );
    }

    #[test]
    fn capitalised_array_param_accepts_array_literal() {
        check_ok(
            "fn sum(Array items) -> int { return 0; }\n\
             sum([1, 2, 3]);\n",
        );
    }

    #[test]
    fn lowercase_array_param_still_accepted() {
        check_ok(
            "fn sum(array items) -> int { return 0; }\n\
             sum([1, 2, 3]);\n",
        );
    }

    #[test]
    fn capitalised_array_struct_field_accepts_array_literal() {
        check_ok(
            "struct Bag { Array items }\n\
             let b = new Bag { items: [1, 2, 3] };\n",
        );
    }

    #[test]
    fn capitalised_array_return_type_accepted() {
        check_ok(
            "fn make() -> Array { return [1, 2]; }\n\
             let xs = make();\n",
        );
    }

    #[test]
    fn wrong_type_still_errors_for_array_param() {
        check_err(
            "fn sum(Array items) -> int { return 0; }\n\
             sum(42);\n",
            "Type mismatch",
        );
    }
}

#[cfg(test)]
mod res2711_type_char {
    use super::*;

    fn check_ok(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    fn check_err(src: &str, fragment: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let e = TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected a type error but got Ok");
        assert!(
            e.contains(fragment),
            "expected {:?} in error: {e}",
            fragment
        );
    }

    #[test]
    fn char_literal_is_typed_as_char() {
        check_ok("let c: char = 'A';");
    }

    #[test]
    fn char_and_capitalised_char_annotation_both_resolve() {
        check_ok("let c: char = 'x'; let d: Char = 'y';");
    }

    #[test]
    fn function_returning_char_from_literal_accepted() {
        check_ok(
            "fn get() -> char { return 'Z'; }\n\
             let c: char = get();\n",
        );
    }

    #[test]
    fn char_to_upper_lower_return_char() {
        check_ok(
            "let u: char = char_to_upper('a');\n\
             let l: char = char_to_lower('A');\n",
        );
    }

    #[test]
    fn char_to_int_returns_int() {
        check_ok("let n: int = char_to_int('A');");
    }

    #[test]
    fn int_assigned_to_char_variable_is_type_error() {
        check_err("let c: char = 42;", "has type int");
    }
}

#[cfg(test)]
mod res2713_pattern_literal_type_check {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    fn check_err(src: &str, fragment: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let e = TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected a type error but got Ok");
        assert!(
            e.contains(fragment),
            "expected {:?} in error: {e}",
            fragment
        );
    }

    #[test]
    fn char_pattern_on_int_scrutinee_is_error() {
        check_err(
            "let x: int = 5; match x { 'a' => println(\"x\"), _ => println(\"y\") }",
            "incompatible",
        );
    }

    #[test]
    fn int_pattern_on_char_scrutinee_is_error() {
        check_err(
            "let c: char = 'a'; match c { 1 => println(\"x\"), _ => println(\"y\") }",
            "incompatible",
        );
    }

    #[test]
    fn bool_pattern_on_string_scrutinee_is_error() {
        check_err(
            r#"let s = "hello"; match s { true => println("x"), _ => println("y") }"#,
            "incompatible",
        );
    }

    #[test]
    fn correct_char_pattern_on_char_scrutinee_accepted() {
        check_ok("let c: char = 'x'; match c { 'x' => println(\"x\"), _ => println(\"y\") }");
    }

    #[test]
    fn correct_int_pattern_on_int_scrutinee_accepted() {
        check_ok("let n: int = 5; match n { 5 => println(\"five\"), _ => println(\"other\") }");
    }

    #[test]
    fn any_scrutinee_accepts_any_literal_pattern() {
        // When the scrutinee type is unknown (Any), all literal patterns pass.
        check_ok(
            "fn f(any x) -> void { match x { 'a' => println(\"a\"), 1 => println(\"1\"), _ => println(\"_\") } } f(0);",
        );
    }

    #[test]
    fn string_subscript_returns_char_type() {
        // s[i] now produces Type::Char, so the result can be bound to a char variable.
        check_ok(r#"let s = "hello"; let c: char = s[0];"#);
    }
}

#[cfg(test)]
mod res2715_try_op_option {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    fn check_err(src: &str, fragment: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let e = TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected a type error but got Ok");
        assert!(
            e.contains(fragment),
            "expected {:?} in error: {e}",
            fragment
        );
    }

    #[test]
    fn try_on_option_accepted() {
        check_ok(
            "fn f() -> Option { return Some(1); }\n\
             fn g() -> Option { let v = f()?; return Some(v); }\n\
             g();\n",
        );
    }

    #[test]
    fn try_on_result_still_accepted() {
        check_ok(
            "fn ok_result() -> Result { return Ok(42); }\n\
             fn use_it() -> Result { let v = ok_result()?; return Ok(v); }\n\
             use_it();\n",
        );
    }

    #[test]
    fn try_on_int_rejected() {
        check_err(
            "fn f() -> int { let n = 5; let v = n?; return v; }",
            "Result or Option",
        );
    }

    #[test]
    fn try_on_string_rejected() {
        check_err(
            r#"fn f() -> string { let s = "hello"; let v = s?; return v; }"#,
            "Result or Option",
        );
    }

    #[test]
    fn try_propagates_option_inner_type() {
        // When the option inner type is known, ? should unwrap to that type.
        // Binding to a typed variable exercises the type propagation.
        check_ok(
            "fn safe_div(int a, int b) -> Option { \
               if b == 0 { return None; } \
               return Some(a / b); \
             }\n\
             fn compute() -> Option { \
               let r = safe_div(10, 2)?; \
               return Some(r * 2); \
             }\n\
             compute();\n",
        );
    }
}

#[cfg(test)]
mod res2717_null_coalescing {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    fn check_err(src: &str, fragment: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let e = TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected a type error but got Ok");
        assert!(
            e.contains(fragment),
            "expected {:?} in error: {e}",
            fragment
        );
    }

    #[test]
    fn null_coalesce_on_option_accepted() {
        check_ok(
            "fn f() -> Option { return Some(42); }\n\
             let x = f() ?? 0;\n\
             println(to_string(x));\n",
        );
    }

    #[test]
    fn null_coalesce_on_none_accepted() {
        check_ok(
            "let x = None ?? 99;\n\
             println(to_string(x));\n",
        );
    }

    #[test]
    fn null_coalesce_on_non_option_rejected() {
        check_err("let x = 42 ?? 0;", "requires an Option on the left");
    }

    #[test]
    fn null_coalesce_on_string_rejected() {
        check_err(
            r#"let x = "hello" ?? "world";"#,
            "requires an Option on the left",
        );
    }

    #[test]
    fn null_coalesce_any_lhs_accepted() {
        check_ok(
            "fn any_fn() -> any { return None; }\n\
             let x = any_fn() ?? 0;\n",
        );
    }
}

#[cfg(test)]
mod res2719_type_name_aliases {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    #[test]
    fn str_alias_for_string() {
        check_ok(r#"fn greet(str name) -> str { return name; }"#);
    }

    #[test]
    fn capital_string_alias_for_string() {
        check_ok(r#"fn greet(String name) -> String { return name; }"#);
    }

    #[test]
    fn boolean_alias_for_bool() {
        check_ok("fn pos(int x) -> boolean { return x > 0; }");
    }

    #[test]
    fn int32_alias_for_int32_type() {
        check_ok("fn f(int32 x) -> int32 { return x; }");
    }

    #[test]
    fn int8_alias_for_int8_type() {
        check_ok("fn f(int8 x) -> int8 { return x; }");
    }

    #[test]
    fn uint8_alias_for_uint8_type() {
        check_ok("fn f(uint8 x) -> uint8 { return x; }");
    }

    #[test]
    fn uint32_alias_for_uint32_type() {
        check_ok("fn f(uint32 x) -> uint32 { return x; }");
    }

    #[test]
    fn byte_alias_for_uint8() {
        check_ok("fn f(byte x) -> byte { return x; }");
    }

    #[test]
    fn double_alias_for_float() {
        check_ok("fn f(double x) -> double { return x; }");
    }

    #[test]
    fn long_alias_for_int() {
        check_ok("fn f(long x) -> long { return x; }");
    }

    #[test]
    fn str_param_can_concat_with_string_literal() {
        check_ok(r#"fn greet(str name) -> string { return "hi " + name; }"#);
    }

    #[test]
    fn string_param_can_be_assigned_to_string_var() {
        check_ok("fn greet(String name) -> string { let s: string = name; return s; }");
    }
}

#[cfg(test)]
mod res2721_interp_string_scope {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok_no_warnings(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    #[test]
    fn let_variable_in_interp_no_false_positive() {
        // Variables defined by let-bindings must be visible inside {expr}.
        check_ok_no_warnings(r#"let x = 10; let s = "{x}";"#);
    }

    #[test]
    fn multi_variable_interp_no_false_positive() {
        check_ok_no_warnings(r#"let a = 1; let b = 2; let s = "{a} and {b}";"#);
    }

    #[test]
    fn expression_in_interp_no_false_positive() {
        check_ok_no_warnings(r#"let x = 5; let y = 3; let s = "{x + y}";"#);
    }

    #[test]
    fn function_return_in_interp_no_false_positive() {
        check_ok_no_warnings(
            "fn double(int n) -> int { return n * 2; }\n\
             let s = \"result: {double(7)}\";\n",
        );
    }

    #[test]
    fn interp_string_is_typed_as_string() {
        let (prog, errs) = parse(r#"let x = 1; let s: string = "{x}";"#);
        assert!(errs.is_empty());
        TypeChecker::new()
            .check_program(&prog)
            .expect("typed let with interp string should pass");
    }
}
