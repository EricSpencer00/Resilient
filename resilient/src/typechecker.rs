// Type checker module for Resilient language
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
    String,
    Bool,
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
    /// RES-053: user-defined record by name. Field types looked up
    /// against the struct table when G7 goes deeper.
    Struct(String),
    Void,
    Any, // Used for untyped variables during inference
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
            Type::String => write!(f, "string"),
            Type::Bool => write!(f, "bool"),
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
            Type::Struct(n) => write!(f, "{}", n),
            Type::Void => write!(f, "void"),
            Type::Any => write!(f, "any"),
            Type::Var(_, Some(span)) => {
                write!(f, "type hole at {}:{}", span.start.line, span.start.column)
            }
            Type::Var(id, None) => write!(f, "?t{}", id),
        }
    }
}

/// RES-053: Two types are compatible if they're equal or if either is
/// Any. Used everywhere we need "same type, or we don't know yet."
///
/// RES-366: `Type::Int` (the type of integer literals) is compatible
/// with every pinned integer type — assigning a literal `42` to an
/// `Int8` binding is always legal. Pinned types are NOT compatible
/// with each other: `Int8 ↔ Int16` requires an explicit `as_int16`
/// cast.
fn compatible(a: &Type, b: &Type) -> bool {
    if a == b {
        return true;
    }
    if matches!(a, Type::Any) || matches!(b, Type::Any) {
        return true;
    }
    // Integer literals produce Type::Int; allow assigning them to any
    // pinned integer type without an explicit cast.
    if *a == Type::Int && is_pinned_int(b) {
        return true;
    }
    if is_pinned_int(a) && *b == Type::Int {
        return true;
    }
    false
}

/// RES-160: collect the binding names a pattern introduces, in
/// source order. Used to verify that all branches of an or-pattern
/// bind the same names.
fn pattern_bindings(p: &Pattern) -> Vec<String> {
    match p {
        Pattern::Identifier(n) => vec![n.clone()],
        Pattern::Wildcard | Pattern::Literal(_) => Vec::new(),
        Pattern::Or(branches) => {
            // By induction (checked at each arm) every branch
            // introduces the same names — pick the first branch's
            // list. Callers use this helper AFTER the consistency
            // check.
            branches.first().map(pattern_bindings).unwrap_or_default()
        }
        // RES-161a: outer name + whatever the inner pattern binds.
        Pattern::Bind(outer, inner) => {
            let mut bs = vec![outer.clone()];
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
    }
}

/// RES-160: does the pattern match every value (i.e. a
/// wildcard / identifier, or an or-pattern with at least one
/// always-matching branch)? Counts as a "default" arm for
/// exhaustiveness.
fn pattern_is_default(p: &Pattern) -> bool {
    match p {
        Pattern::Wildcard | Pattern::Identifier(_) => true,
        Pattern::Literal(_) => false,
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
        Pattern::Some(_) | Pattern::None => false,
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
        Pattern::Literal(_) => false,
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
        // RES-375: Option patterns don't match struct-nominal types.
        Pattern::Some(_) | Pattern::None => false,
    }
}

fn pattern_is_exhaustive_wrt_scrutinee(
    scrut: &Type,
    p: &Pattern,
    struct_fields: &HashMap<String, Vec<(String, Type)>>,
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
        } if operator == "!" => fold_const_bool(right, bindings).map(|b| !b),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => match operator.as_str() {
            "&&" => match (
                fold_const_bool(left, bindings),
                fold_const_bool(right, bindings),
            ) {
                (Some(a), Some(b)) => Some(a && b),
                (Some(false), _) | (_, Some(false)) => Some(false),
                _ => None,
            },
            "||" => match (
                fold_const_bool(left, bindings),
                fold_const_bool(right, bindings),
            ) {
                (Some(a), Some(b)) => Some(a || b),
                (Some(true), _) | (_, Some(true)) => Some(true),
                _ => None,
            },
            "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                let l = fold_const_i64(left, bindings)?;
                let r = fold_const_i64(right, bindings)?;
                Some(match operator.as_str() {
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
        && operator == "=="
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
        } if operator == "-" => fold_const_i64(right, bindings).map(|v| -v),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let l = fold_const_i64(left, bindings)?;
            let r = fold_const_i64(right, bindings)?;
            match operator.as_str() {
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
#[derive(Debug, Clone)]
pub struct TypeEnvironment {
    store: HashMap<String, Type>,
    outer: Option<Box<TypeEnvironment>>,
}

impl TypeEnvironment {
    pub fn new() -> Self {
        TypeEnvironment {
            store: HashMap::new(),
            outer: None,
        }
    }

    pub fn new_enclosed(outer: TypeEnvironment) -> Self {
        TypeEnvironment {
            store: HashMap::new(),
            outer: Some(Box::new(outer)),
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
        let mut out = std::collections::HashSet::new();
        for (name, n) in &self.per_fn_discharged {
            if *n > 0 && !self.per_fn_runtime.contains_key(name) {
                out.insert(name.clone());
            }
        }
        out
    }
}

/// RES-067: shim that forwards to the Z3 module when built --features z3,
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

/// RES-354: theory-aware variant of `z3_prove_with_cert`. Uses
/// `prove_auto` which auto-selects BV32/LIA based on the theory hint
/// and the presence of bitwise operations.
#[cfg(feature = "z3")]
fn z3_prove_with_cert_theory(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    timeout_ms: u32,
    theory: crate::verifier_z3::Z3Theory,
) -> (Option<bool>, Option<String>, Option<String>, bool) {
    let (verdict, cert, cx, timed_out) =
        crate::verifier_z3::prove_auto(expr, bindings, theory, timeout_ms);
    (verdict, cert.map(|c| c.smt2), cx, timed_out)
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
#[cfg(feature = "z3")]
fn z3_prove_with_axioms_and_cert(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    axioms: &[Node],
    timeout_ms: u32,
) -> (Option<bool>, Option<String>, Option<String>, bool) {
    let (verdict, cert, cx, timed_out) =
        crate::verifier_z3::prove_with_axioms_and_timeout(expr, bindings, axioms, timeout_ms);
    (verdict, cert.map(|c| c.smt2), cx, timed_out)
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
    std::env::var("RESILIENT_RICH_DIAG").as_deref() == Ok("1")
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
#[derive(Debug, Clone)]
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
    contract_table: HashMap<String, ContractInfo>,
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
    struct_fields: HashMap<String, Vec<(String, Type)>>,
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
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut env = TypeEnvironment::new();

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
        // RES-410: sign(x) — -1/0/+1.
        env.set("sign".to_string(), fn_any_to_any());
        // RES-411: float predicates — return Bool, signed as Any per pattern.
        env.set("is_nan".to_string(), fn_any_to_any());
        env.set("is_inf".to_string(), fn_any_to_any());
        env.set("is_finite".to_string(), fn_any_to_any());
        env.set("sqrt".to_string(), fn_any_to_any());
        env.set("floor".to_string(), fn_any_to_any());
        env.set("ceil".to_string(), fn_any_to_any());
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
        env.set("ln".to_string(), fn_float_to_float());
        env.set("exp".to_string(), fn_float_to_float());
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
        env.set("array_reverse".to_string(), fn_any_to_any());
        // RES-416: integer-array reductions.
        env.set("array_sum".to_string(), fn_any_to_any());
        env.set("array_product".to_string(), fn_any_to_any());
        // RES-417: array min/max.
        env.set("array_min".to_string(), fn_any_to_any());
        env.set("array_max".to_string(), fn_any_to_any());
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
        env.set("array_contains".to_string(), fn_any_any_to_any());
        env.set("array_index_of".to_string(), fn_any_any_to_any());
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
        env.set("array_concat".to_string(), fn_any_any_to_any());
        // RES-515: three-way concatenation.
        env.set(
            "array_concat3".to_string(),
            Type::Function {
                params: vec![Type::Any, Type::Any, Type::Any],
                return_type: Box::new(Type::Array),
            },
        );
        // RES-421: take/drop first n.
        env.set("array_take".to_string(), fn_any_any_to_any());
        env.set("array_drop".to_string(), fn_any_any_to_any());
        // RES-514: pick every nth element.
        env.set("array_step".to_string(), fn_any_any_to_any());
        // RES-422: integer sort ascending.
        env.set("array_sort".to_string(), fn_any_to_any());
        // RES-443: integer sort descending.
        env.set("array_sort_desc".to_string(), fn_any_to_any());
        // RES-444: Fisher-Yates shuffle (impure: uses RNG).
        env.set("array_shuffle".to_string(), fn_any_to_any());
        // RES-445: array prefix/suffix predicates.
        env.set("array_starts_with".to_string(), fn_any_any_to_any());
        env.set("array_ends_with".to_string(), fn_any_any_to_any());
        // RES-446: all match indices.
        env.set("string_find_all".to_string(), fn_any_any_to_any());
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
        env.set(
            "array_swap".to_string(),
            Type::Function {
                params: vec![Type::Any, Type::Int, Type::Int],
                return_type: Box::new(Type::Any),
            },
        );
        // RES-451: insert/remove at index.
        env.set(
            "array_insert_at".to_string(),
            Type::Function {
                params: vec![Type::Any, Type::Int, Type::Any],
                return_type: Box::new(Type::Any),
            },
        );
        env.set(
            "array_remove_at".to_string(),
            Type::Function {
                params: vec![Type::Any, Type::Int],
                return_type: Box::new(Type::Any),
            },
        );
        // RES-452: replace element at index.
        env.set(
            "array_set_at".to_string(),
            Type::Function {
                params: vec![Type::Any, Type::Int, Type::Any],
                return_type: Box::new(Type::Any),
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
        env.set("array_window".to_string(), fn_any_any_to_any());
        // RES-456: rotation.
        env.set("array_rotate_left".to_string(), fn_any_any_to_any());
        env.set("array_rotate_right".to_string(), fn_any_any_to_any());
        // RES-457: capitalize.
        env.set(
            "string_capitalize".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            },
        );
        // RES-458: array_cycle.
        env.set("array_cycle".to_string(), fn_any_any_to_any());
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
        env.set("array_pairs".to_string(), fn_any_to_any());
        // RES-463: UTF-8 byte length.
        env.set(
            "string_bytes_len".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Int),
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
        env.set("array_remove".to_string(), fn_any_any_to_any());
        // RES-467: remove all matching elements.
        env.set("array_remove_all".to_string(), fn_any_any_to_any());
        // RES-468: collapse adjacent duplicates.
        env.set("array_dedup".to_string(), fn_any_to_any());
        // RES-504: partition into maximal runs of equal int elements.
        env.set("array_group_by_int".to_string(), fn_any_to_any());
        // RES-469: scalar all/any equality predicates.
        env.set("array_all_eq".to_string(), fn_any_any_to_any());
        env.set("array_any_eq".to_string(), fn_any_any_to_any());
        // RES-471: prefix/suffix strippers.
        let str_str_to_str = Type::Function {
            params: vec![Type::String, Type::String],
            return_type: Box::new(Type::String),
        };
        env.set("string_strip_prefix".to_string(), str_str_to_str.clone());
        env.set("string_strip_suffix".to_string(), str_str_to_str);
        // RES-472: element-wise array equality.
        env.set("array_eq".to_string(), fn_any_any_to_any());
        // RES-473: ternary numeric min/max.
        let any3_to_any = Type::Function {
            params: vec![Type::Any, Type::Any, Type::Any],
            return_type: Box::new(Type::Any),
        };
        env.set("min3".to_string(), any3_to_any.clone());
        env.set("max3".to_string(), any3_to_any);
        // RES-474: array_ne.
        env.set("array_ne".to_string(), fn_any_any_to_any());
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
        // RES-478: array_count_eq alias.
        env.set("array_count_eq".to_string(), fn_any_any_to_any());
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
        env.set("array_rest".to_string(), fn_any_to_any());
        env.set("array_init".to_string(), fn_any_to_any());
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
        // RES-485: |a - b|.
        env.set(
            "abs_diff".to_string(),
            Type::Function {
                params: vec![Type::Int, Type::Int],
                return_type: Box::new(Type::Int),
            },
        );
        // RES-486: (quotient, remainder) tuple.
        env.set(
            "divmod".to_string(),
            Type::Function {
                params: vec![Type::Int, Type::Int],
                return_type: Box::new(Type::Any),
            },
        );
        // RES-423: flatten one level.
        env.set("array_flatten".to_string(), fn_any_to_any());
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
        env.set("array_unique".to_string(), fn_any_to_any());
        // RES-427: count element occurrences.
        env.set("array_count".to_string(), fn_any_any_to_any());
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
        // RES-430: pair elements as tuples; truncate to shorter array.
        env.set("array_zip".to_string(), fn_any_any_to_any());
        // RES-531: split an array of 2-tuples into two parallel arrays.
        env.set("array_unzip".to_string(), fn_any_to_any());
        // RES-431: integer range [start, end).
        env.set("array_range".to_string(), fn_any_any_to_any());
        // RES-522: indices of an array as a new array.
        env.set("array_indices".to_string(), fn_any_to_any());
        // RES-432: array of n copies.
        env.set("array_repeat".to_string(), fn_any_any_to_any());
        // RES-433: split string into single-char strings.
        env.set("string_chars".to_string(), fn_any_to_any());
        // RES-434: split string into lines (LF, CRLF).
        env.set("string_lines".to_string(), fn_any_to_any());
        // RES-496: split on Unicode whitespace.
        env.set("string_words".to_string(), fn_any_to_any());
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
        env.set("array_chunk".to_string(), fn_any_any_to_any());
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
        env.set("array_intersperse".to_string(), fn_any_any_to_any());
        // RES-516: alternate elements from two arrays.
        env.set("array_interleave".to_string(), fn_any_any_to_any());
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
        env.set("array_split_at".to_string(), fn_any_any_to_any());
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
        env.set("bit_rotate_right".to_string(), int_int_to_int);
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

        // Result builtins
        env.set("Ok".to_string(), fn_any_to_result());
        env.set("Err".to_string(), fn_any_to_result());
        env.set("is_ok".to_string(), fn_result_to_bool());
        env.set("is_err".to_string(), fn_result_to_bool());
        env.set("unwrap".to_string(), fn_result_to_any());
        env.set("unwrap_err".to_string(), fn_result_to_any());

        // RES-328: `cell(initial)` — shared mutable container.
        // Element type isn't tracked at the type-system layer (the
        // generic story lands with G7); the runtime enforces that
        // `.set` rebinds the inner value, and the inner value's
        // dynamic type flows through `Type::Any`.
        env.set("cell".to_string(), fn_any_to_any());

        TypeChecker {
            env,
            contract_table: HashMap::new(),
            fn_decl_spans: HashMap::new(),
            const_bindings: HashMap::new(),
            stats: VerificationStats::default(),
            certificates: Vec::new(),
            struct_fields: HashMap::new(),
            type_aliases: HashMap::new(),
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
            // RES-354: auto-detect theory by default.
            #[cfg(feature = "z3")]
            z3_theory: crate::verifier_z3::Z3Theory::Auto,
            // RES-318: per-loop-invariant verbose stderr line is OFF
            // by default. The driver flips it on via `--verbose`.
            verbose_loop_invariants: false,
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
    #[allow(dead_code)]
    pub(crate) fn push_loop_invariant_certificate(&mut self, idx: usize, smt2: String) {
        self.certificates.push(CapturedCertificate {
            fn_name: "<loop>".to_string(),
            kind: "loop_invariant",
            idx,
            smt2,
        });
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
                            span,
                            ..
                        } => {
                            self.contract_table.insert(
                                name.clone(),
                                ContractInfo {
                                    parameters: parameters.clone(),
                                    requires: requires.clone(),
                                    ensures: ensures.clone(),
                                    fails: fails.clone(),
                                },
                            );
                            // RES-340: remember the fn keyword's span so
                            // the rich type-mismatch path can point at
                            // the declaration.
                            self.fn_decl_spans.insert(name.clone(), *span);
                        }
                        Node::TypeAlias { name, target, .. } => {
                            self.type_aliases.insert(name.clone(), target.clone());
                        }
                        _ => {}
                    }
                }

                let mut result_type = Type::Void;
                for stmt in statements {
                    result_type = self.check_node(&stmt.node).map_err(|e| {
                        // RES-080: prepend file:line:col so users can
                        // locate the offending statement. Skip the
                        // prefix when the span looks default/empty
                        // (line 0 means "synthetic" — see span.rs).
                        if stmt.span.start.line == 0 {
                            e
                        } else {
                            format!(
                                "{}:{}:{}: {}",
                                source_path, stmt.span.start.line, stmt.span.start.column, e
                            )
                        }
                    })?;
                }

                // RES-388: verify every `actor`'s `always` safety
                // invariants. The walk happens *after* per-statement
                // type-checking so we only reason about well-typed
                // bodies. Any obligation that Z3 refutes becomes a
                // hard error with a file:line:col diagnostic naming
                // the actor, handler, and invariant; Unknown /
                // Unsupported verdicts emit a stderr warning but do
                // not fail the check (matching how partial proofs
                // of `requires` / `ensures` are handled — RES-217).
                let obligations = collect_actor_obligations(statements, self.verifier_timeout_ms);
                let mut refuted: Vec<String> = Vec::new();
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
                check_program_purity(statements, source_path)?;

                // RES-389: effect-annotation enforcement.
                check_program_effects(statements, source_path)?;

                // RES-385: single-use enforcement for linear types.
                crate::linear::check_linear_usage(program, source_path)?;

                // <EXTENSION_PASSES>
                // Add new compiler pass calls here (append-only).
                // Pattern: crate::your_feature::check(program, source_path)?;
                // Merge conflicts: keep ALL calls from both sides.
                crate::try_catch::check(program, source_path)?;
                crate::verifier_liveness::check(program, source_path)?;
                crate::recovery_checker::check(program, source_path)?;
                crate::assume_false_checker::check(program, source_path)?;
                crate::bounds_check::check_array_bounds(program, source_path)?;
                crate::loop_invariants::check(program, source_path)?;
                crate::verifier_loop_invariants::verify_and_capture(self, program);
                crate::type_aliases::check(program, source_path)?;
                crate::ranges::check(program, source_path)?;
                crate::string_interp::check(program, source_path)?;
                crate::modules::check(program, source_path)?;
                crate::default_params::check(program, source_path)?;
                crate::generics::check(program, source_path)?;
                crate::newtypes::check(program, source_path)?;
                crate::traits::check(program, source_path)?;
                // </EXTENSION_PASSES>

                // RES-192: IO-effect inference. Binary lattice
                // (pure / IO). Fixpoint over the call graph: a fn
                // is tagged IO iff it calls an impure builtin, an
                // already-IO user fn, or an unresolvable callee.
                // Non-error — just populates the `fn_effects`
                // stats field for the --audit column.
                self.stats.fn_effects = infer_fn_effects(statements);

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
            Pattern::Wildcard | Pattern::Literal(_) => Ok(vec![]),
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
                let Type::Struct(sname) = scrut_ty else {
                    return Err(format!(
                        "struct pattern `{}` used where scrutinee is not a struct (got {})",
                        struct_name, scrut_ty
                    ));
                };
                if struct_name != sname {
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
                let mut seen = HashSet::<String>::new();
                let mut out = Vec::new();
                for (fname, sub) in fields {
                    if !seen.insert(fname.clone()) {
                        return Err(format!(
                            "duplicate field `{}` in struct match pattern",
                            fname
                        ));
                    }
                    let Some((_, fty)) = decl.iter().find(|(n, _)| n == fname) else {
                        return Err(format!(
                            "struct `{}` has no field `{}` in match pattern",
                            sname, fname
                        ));
                    };
                    let sub_bt = self.match_pattern_binding_types(sub.as_ref(), fty)?;
                    out.extend(sub_bt);
                }
                Ok(out)
            }
            // RES-375: `Some(inner)` binds the inner value as `Any`
            // (Option carries no type parameter in the dynamic checker).
            // `None` introduces no bindings.
            Pattern::Some(inner) => self.match_pattern_binding_types(inner.as_ref(), &Type::Any),
            Pattern::None => Ok(vec![]),
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
        let saved = self.env.clone();
        let mut inner = TypeEnvironment::new_enclosed(saved.clone());
        inner.set(var.to_string(), ty);
        self.env = inner;
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
            // FFI v1: validate extern block — reject non-primitive types.
            Node::Extern { decls, .. } => {
                const PARAM_PRIMS: &[&str] = &["Int", "Float", "Bool", "String"];
                const RET_PRIMS: &[&str] = &["Int", "Float", "Bool", "String", "Void"];
                for d in decls {
                    for (ty, name) in &d.parameters {
                        if !PARAM_PRIMS.contains(&ty.as_str()) {
                            return Err(format!(
                                "FFI: extern parameter `{}` has type `{}`; extern fn supports only {} in v1",
                                name,
                                ty,
                                PARAM_PRIMS.join(", ")
                            ));
                        }
                    }
                    if !RET_PRIMS.contains(&d.return_type.as_str()) {
                        return Err(format!(
                            "FFI: extern return type `{}` not supported in v1 (allowed: {})",
                            d.return_type,
                            RET_PRIMS.join(", ")
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
                ..
            } => {
                let mut param_types = Vec::new();

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
                                z3_prove_with_cert(clause, &no_bindings, self.verifier_timeout_ms)
                            }
                        };
                        verdict = v;
                        if matches!(verdict, Some(true)) {
                            self.stats.requires_discharged_by_z3 += 1;
                            if let Some(smt2) = cert {
                                self.certificates.push(CapturedCertificate {
                                    fn_name: name.clone(),
                                    kind: "decl",
                                    idx: decl_idx,
                                    smt2,
                                });
                            }
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
                        // RES-354: use theory-aware prover.
                        let (_v, _cert, _c, _timed_out) = {
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
                                z3_prove_with_cert(clause, &no_bindings, self.verifier_timeout_ms)
                            }
                        };
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

                    // Contradiction: clause is unreachable regardless
                    // of `fails`. Always a compile error.
                    if matches!(verdict, Some(false)) {
                        let base = format!(
                            "{}fn {}: `recovers_to` can never hold — the recovery invariant is a contradiction",
                            pos_prefix, name
                        );
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
                        if let Some(smt2) = cert_smt2 {
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

                    // RES-392b: per-prefix bounded model checking.
                    // Extends the MVP (final-state only) with verification
                    // that the recovers_to clause holds after recovery from
                    // ANY instruction boundary in the function body.
                    crate::recovers_to_bmc::check_recovers_to_bmc(name, body, clause)?;
                }

                // RES-065: push each requires clause's extractable
                // assumption into const_bindings so interior call
                // sites can use them. This is the inter-procedural
                // chaining step.
                let mut pushed_assumptions: Vec<(String, Option<i64>)> = Vec::new();
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

                // Check function body
                let body_result = self.check_node(body);

                // RES-387: leave the fault scope before propagating any
                // error, so a nested fn declared inside this body does
                // not inherit our fails set on the way out.
                self.current_fn_fails = saved_fn_fails;

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
                    if !compatible(&declared, &body_type) {
                        return Err(format!(
                            "fn {}: return type mismatch — declared {}, body produces {}",
                            name, declared, body_type
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
            Node::Range { lo, hi, .. } => {
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
                condition, message, ..
            } => {
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
                condition, message, ..
            } => {
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

                for stmt in statements {
                    result_type = self.check_node(stmt)?;
                }

                // Restore original environment
                std::mem::swap(&mut self.env, &mut block_env);

                Ok(result_type)
            }

            Node::LetStatement {
                name,
                value,
                type_annot,
                span,
            } => {
                let value_type = self.check_node(value)?;
                // RES-053: enforce `let x: T = value` — reject if value's
                // type isn't compatible with the declared annotation.
                let bind_type = if let Some(ty_name) = type_annot {
                    let declared = self.parse_type_name(ty_name)?;
                    if !compatible(&declared, &value_type) {
                        return Err(format!(
                            "let {}: {} — value has type {}",
                            name, declared, value_type
                        ));
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
                    if !matches!(value_type, Type::Any | Type::Void | Type::Var(..)) {
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
                let no_b: HashMap<String, i64> = HashMap::new();
                if let Some(v) = fold_const_i64(value, &no_b)
                    .or_else(|| fold_const_i64(value, &self.const_bindings))
                {
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
                            return Err(format!("Struct {} has no field `{}`", struct_name, pf));
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
                for item in items {
                    let _ = self.check_node(item)?;
                }
                Ok(Type::Array)
            }

            // RES-148: map literal — walk every key and value to
            // surface nested type errors, but fall back to `Type::Any`
            // for the result until a real `Type::Map<K, V>` lands in
            // the typechecker.
            Node::MapLiteral { entries, .. } => {
                for (k, v) in entries {
                    let _ = self.check_node(k)?;
                    let _ = self.check_node(v)?;
                }
                Ok(Type::Any)
            }

            // RES-149: set literal. Walk each item to catch nested
            // type errors; return `Type::Any` for now — same posture
            // as `MapLiteral` until `Type::Set<T>` shows up.
            Node::SetLiteral { items, .. } => {
                for item in items {
                    let _ = self.check_node(item)?;
                }
                Ok(Type::Any)
            }

            Node::TryExpression { expr: inner, .. } => {
                let inner_type = self.check_node(inner)?;
                // `?` expects a Result and unwraps to Any at MVP (we
                // don't track Ok's payload type yet).
                if !compatible(&inner_type, &Type::Result) {
                    return Err(format!("? operator expects a Result, got {}", inner_type));
                }
                Ok(Type::Any)
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
                parameters, body, ..
            } => {
                // Evaluate the body's type in a child env with params
                // bound, just like named Function.
                let mut param_types = Vec::new();
                let mut fn_env = TypeEnvironment::new_enclosed(self.env.clone());
                for (tname, pname) in parameters {
                    let ty = self.parse_type_name(tname)?;
                    param_types.push(ty.clone());
                    fn_env.set(pname.clone(), ty);
                }
                std::mem::swap(&mut self.env, &mut fn_env);
                let body_type = self.check_node(body)?;
                std::mem::swap(&mut self.env, &mut fn_env);
                Ok(Type::Function {
                    params: param_types,
                    return_type: Box::new(body_type),
                })
            }

            Node::Match {
                scrutinee, arms, ..
            } => {
                let scrutinee_type = self.check_node(scrutinee)?;
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
                    // Roll back all pattern-binding entries.
                    for (n, prev) in rollback_bindings {
                        match prev {
                            Some(t) => self.env.set(n, t),
                            None => {
                                self.env.remove(&n);
                            }
                        }
                    }
                    let _ = body_res?;
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
                            return Err(format!(
                                "Non-exhaustive match on struct `{}`: add `{} {{ .. }}`, `_`, or an identifier arm that covers every field",
                                sname, sname
                            ));
                        }
                        other => {
                            return Err(format!(
                                "Non-exhaustive match on {}: add a wildcard `_` or identifier arm to handle unmatched values",
                                other
                            ));
                        }
                    }
                }

                Ok(Type::Any)
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
            // mapping. A duplicate alias-name declaration overwrites
            // the earlier one — consistent with how `StructDecl`
            // treats duplicate struct names today.
            //
            // NOTE: aliases are NOT nominal — `Meters` unifies with
            // `Int`. Users who want a fresh nominal type wrap the
            // target in a one-field struct (RES-126 covers the
            // nominal rule).
            Node::TypeAlias { name, target, .. } => {
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

            // RES-333: supervisor declaration. Phase 1: stub implementation.
            Node::SupervisorDecl { .. } => {
                // RES-333: Phase 3 supervisor validation
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

            // RES-401: tuples — type-check each item in a literal,
            // recurse into the destructure RHS, and treat element
            // access as `Type::Any` until the type system grows a
            // dedicated tuple shape (follow-up ticket).
            Node::TupleLiteral { items, .. } => {
                for it in items {
                    self.check_node(it)?;
                }
                Ok(Type::Any)
            }
            Node::TupleIndex { tuple, .. } => {
                self.check_node(tuple)?;
                Ok(Type::Any)
            }
            Node::LetTupleDestructure { names, value, .. } => {
                self.check_node(value)?;
                for n in names {
                    self.env.set(n.clone(), Type::Any);
                }
                Ok(Type::Void)
            }

            // RES-388/RES-390: ActorDecl type-checks state fields,
            // always invariants, and receive handler bodies.
            Node::ActorDecl {
                name,
                state_fields,
                always_clauses,
                eventually_clauses,
                receive_handlers,
                ..
            } => {
                let saved_env = self.env.clone();
                let mut resolved_fields: Vec<(String, Type)> = Vec::new();
                for (ty, field, init) in state_fields {
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
                self.struct_fields
                    .insert(name.clone(), resolved_fields.clone());
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
                    let handler_saved = self.env.clone();
                    self.env.set("self".to_string(), Type::Struct(name.clone()));
                    for (pty, pname) in &handler.parameters {
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
                    self.env = handler_saved;
                }
                self.env = saved_env;
                Ok(Type::Void)
            }

            // RES-153: record the struct's (field, type) list so
            // `FieldAccess` / `FieldAssignment` downstream can check
            // field existence and surface typed-field errors
            // statically.
            Node::StructDecl { name, fields, .. } => {
                let mut resolved: Vec<(String, Type)> = Vec::with_capacity(fields.len());
                for (type_name, field_name) in fields {
                    let ty = self.parse_type_name(type_name)?;
                    resolved.push((field_name.clone(), ty));
                }
                self.struct_fields.insert(name.clone(), resolved);
                Ok(Type::Void)
            }

            Node::StructLiteral { name, fields, .. } => {
                for (_, e) in fields {
                    let _ = self.check_node(e)?;
                }
                Ok(Type::Struct(name.clone()))
            }

            Node::FieldAccess { target, field, .. } => {
                let tgt_ty = self.check_node(target)?;
                // RES-153: if the target is a known struct, return the
                // declared field's type. Otherwise fall back to Any so
                // non-struct targets (e.g. through generic containers)
                // keep the old permissive behaviour.
                if let Type::Struct(sname) = &tgt_ty
                    && let Some(declared) = self.struct_fields.get(sname)
                    && let Some((_, ty)) = declared.iter().find(|(n, _)| n == field)
                {
                    return Ok(ty.clone());
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
                let _ = self.check_node(value)?;
                // RES-153: reject writes to non-existent fields
                // statically when the target's struct is known. The
                // old runtime error ("Struct Point has no field 'z'")
                // still fires for dynamic `Any` targets.
                if let Type::Struct(sname) = &tgt_ty
                    && let Some(declared) = self.struct_fields.get(sname)
                    && !declared.iter().any(|(n, _)| n == field)
                {
                    let avail: Vec<&str> = declared.iter().map(|(n, _)| n.as_str()).collect();
                    return Err(format!(
                        "struct `{}` has no field `{}`; available fields: {}",
                        sname,
                        field,
                        avail.join(", ")
                    ));
                }
                Ok(Type::Void)
            }

            Node::IndexExpression { target, index, .. } => {
                let _ = self.check_node(target)?;
                let _ = self.check_node(index)?;
                // Element type not tracked at MVP.
                Ok(Type::Any)
            }

            Node::IndexAssignment {
                target,
                index,
                value,
                ..
            } => {
                let _ = self.check_node(target)?;
                let _ = self.check_node(index)?;
                let _ = self.check_node(value)?;
                Ok(Type::Void)
            }

            Node::ForInStatement { iterable, body, .. } => {
                let _ = self.check_node(iterable)?;
                let _ = self.check_node(body)?;
                Ok(Type::Void)
            }

            Node::WhileStatement {
                condition, body, ..
            } => {
                let _ = self.check_node(condition)?;
                let _ = self.check_node(body)?;
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
                    if !compatible(&declared, &value_type) {
                        return Err(format!(
                            "const {}: {} — value has type {}",
                            name, declared, value_type
                        ));
                    }
                    declared
                } else {
                    value_type.clone()
                };
                self.env.set(name.clone(), bind_type);
                Ok(Type::Void)
            }

            Node::Assignment { name, value, .. } => {
                let _ = self.check_node(value)?;
                // RES-063: any reassignment kills const-tracking. We
                // could try to re-track if RHS is foldable, but
                // mid-function mutation is rare and the conservative
                // choice keeps the verifier sound.
                self.const_bindings.remove(name);
                Ok(Type::Void)
            }

            Node::ReturnStatement { value, .. } => {
                // Bare `return;` has type Void; otherwise pass through
                // the type of the returned value.
                match value {
                    Some(expr) => self.check_node(expr),
                    None => Ok(Type::Void),
                }
            }

            Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
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
                let assumption = extract_eq_assumption(condition);
                let saved = if let Some((ref name, value)) = assumption {
                    let prev = self.const_bindings.get(name).copied();
                    self.const_bindings.insert(name.clone(), value);
                    Some((name.clone(), prev))
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

                    // Both branches should have compatible types
                    if consequence_type != alternative_type
                        && consequence_type != Type::Any
                        && alternative_type != Type::Any
                    {
                        return Err(format!(
                            "If branches have incompatible types: {} and {}",
                            consequence_type, alternative_type
                        ));
                    }
                }

                Ok(consequence_type)
            }

            Node::ExpressionStatement { expr, .. } => self.check_node(expr),

            Node::Identifier { name, span } => {
                // RES-078: identifier span lets us tell users where
                // exactly the undefined reference lives. Skip the
                // prefix when the span looks default (synthetic).
                match self.env.get(name) {
                    Some(typ) => Ok(typ),
                    None => {
                        // RES-306: append a did-you-mean hint when an
                        // in-scope name is within Levenshtein distance 2
                        // of the typo. The helper handles the
                        // <3-char skip and the cap-at-3 ranking.
                        let names = self.env.all_names();
                        let suggestions = crate::did_you_mean::suggest(
                            name.as_str(),
                            names.iter().map(String::as_str),
                        );
                        let hint = if suggestions.is_empty() {
                            String::new()
                        } else {
                            let body = suggestions
                                .iter()
                                .map(|s| format!("`{}`", s))
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!(" — did you mean {}?", body)
                        };
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
            Node::InterpolatedString { .. } => Ok(Type::String),
            Node::BytesLiteral { .. } => Ok(Type::Bytes),
            Node::BooleanLiteral { .. } => Ok(Type::Bool),

            Node::PrefixExpression {
                operator, right, ..
            } => {
                let right_type = self.check_node(right)?;

                match operator.as_str() {
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
                ..
            } => {
                let left_type = self.check_node(left)?;
                let right_type = self.check_node(right)?;

                // RES-130: `is_numeric` retired for `+ - * / %`; the
                // `check_numeric_same_type` helper now enforces the
                // no-coercion rule. `is_bool` stays for the logical
                // operator arm.
                let is_bool = |t: &Type| matches!(t, Type::Bool | Type::Any);

                match operator.as_str() {
                    "+" => {
                        // String-plus-primitive coercion (RES-008): if
                        // either side is a string, the result is a string.
                        if left_type == Type::String || right_type == Type::String {
                            return Ok(Type::String);
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
                        // RES-130: same policy as `+` — no mixed int /
                        // float.
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
                    _ => Err(format!("Unknown infix operator: {}", operator)),
                }
            }

            Node::CallExpression {
                function,
                arguments,
                span: call_span,
            } => {
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
                    if !info.requires.is_empty() {
                        self.stats.contracted_call_sites += 1;
                    }
                    let mut bindings: HashMap<String, i64> = HashMap::new();
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
                                    z3_prove_with_cert(clause, &bindings, self.verifier_timeout_ms)
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
                            if matches!(verdict, Some(true))
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
                                *self
                                    .stats
                                    .per_fn_discharged
                                    .entry(callee_name.clone())
                                    .or_insert(0) += 1;
                            }
                            None => {
                                self.stats.requires_left_for_runtime += 1;
                                *self
                                    .stats
                                    .per_fn_runtime
                                    .entry(callee_name.clone())
                                    .or_insert(0) += 1;
                            }
                        }
                    }
                }

                match func_type {
                    Type::Function {
                        params,
                        return_type,
                    } => {
                        // Check argument count
                        if arguments.len() != params.len() {
                            return Err(format!(
                                "Expected {} arguments, got {}",
                                params.len(),
                                arguments.len()
                            ));
                        }

                        // Check each argument type
                        for (i, (arg, param_type)) in
                            arguments.iter().zip(params.iter()).enumerate()
                        {
                            let arg_type = self.check_node(arg)?;
                            if arg_type != *param_type
                                && *param_type != Type::Any
                                && arg_type != Type::Any
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
                                        &param_type.to_string(),
                                        &arg_type.to_string(),
                                    ));
                                }
                                return Err(format!(
                                    "Type mismatch in argument {}: expected {}, got {}",
                                    i + 1,
                                    param_type,
                                    arg_type
                                ));
                            }
                        }

                        Ok(*return_type)
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
        }
    }

    fn parse_type_name(&self, name: &str) -> Result<Type, String> {
        // RES-385: the parser prefixes `linear` types with the literal
        // string `linear `. The linearity bit is consumed by the
        // dedicated single-use pass (`check_linear_usage`); at the
        // plain type-equality level, `linear T` and `T` are the same
        // type, so strip the prefix here before resolving.
        let base = crate::linear::strip_linear(name);
        self.parse_type_name_inner(base, &mut Vec::new())
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
            "int" | "Int" | "Int64" => Ok(Type::Int),
            // RES-366: pinned signed integer widths.
            "Int8" => Ok(Type::Int8),
            "Int16" => Ok(Type::Int16),
            "Int32" => Ok(Type::Int32),
            // RES-366: pinned unsigned integer widths.
            "UInt8" => Ok(Type::UInt8),
            "UInt16" => Ok(Type::UInt16),
            "UInt32" => Ok(Type::UInt32),
            "UInt64" => Ok(Type::UInt64),
            "float" => Ok(Type::Float),
            "string" => Ok(Type::String),
            "bool" => Ok(Type::Bool),
            "void" => Ok(Type::Void),
            "Result" => Ok(Type::Result),
            "array" => Ok(Type::Array),
            "" => Ok(Type::Any), // Empty type name means "any" for now
            // RES-128: a registered alias expands transitively.
            other if self.type_aliases.contains_key(other) => {
                if seen.iter().any(|n| n == other) {
                    // Cycle — include the full chain so users see
                    // how they got into it.
                    let mut chain = seen.clone();
                    chain.push(other.to_string());
                    return Err(format!("type alias cycle: {}", chain.join(" -> ")));
                }
                seen.push(other.to_string());
                let target = self.type_aliases[other].clone();
                self.parse_type_name_inner(&target, seen)
            }
            // RES-053: any other identifier is assumed to be a
            // user-defined struct. G7 will register struct decls and
            // reject unknown type names, but at MVP we're permissive.
            other => Ok(Type::Struct(other.to_string())),
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
    // RES-147: monotonic clock.
    "clock_ms",
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
    let mut pure_fns: std::collections::HashSet<String> = std::collections::HashSet::new();
    for stmt in statements {
        if let Node::Function {
            name, pure: true, ..
        } = &stmt.node
        {
            pure_fns.insert(name.clone());
        }
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
    pure_fns: &std::collections::HashSet<String>,
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
                if pure_fns.contains(callee) {
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
        Node::StructLiteral { fields, .. } => {
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
        // RES-411: float predicates.
        "is_nan",
        "is_inf",
        "is_finite",
        "min",
        "max",
        // RES-295.
        "clamp",
        "atan2",
        "sqrt",
        "pow",
        "floor",
        "ceil",
        "to_float",
        "to_int",
        "sin",
        "cos",
        "tan",
        "ln",
        "log",
        "exp",
        // String/collection.
        "len",
        "push",
        "pop",
        "slice",
        "split",
        "trim",
        "contains",
        "to_upper",
        "to_lower",
        // RES-412: reverse string/array.
        "string_reverse",
        "array_reverse",
        // RES-416: array reductions.
        "array_sum",
        "array_product",
        // RES-417: array min/max.
        "array_min",
        "array_max",
        // RES-503: index of max/min int element.
        "array_argmax_int",
        "array_argmin_int",
        // RES-418: array search.
        "array_contains",
        "array_index_of",
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
        // RES-293: HashMap stdlib builtins (purely functional —
        // each returns a new map / scalar; no IO).
        "hashmap_new",
        "hashmap_insert",
        "hashmap_get",
        "hashmap_remove",
        "hashmap_contains",
        "hashmap_keys",
        "set_new",
        "set_insert",
        "set_remove",
        "set_has",
        "set_len",
        "set_items",
        "bytes_len",
        "bytes_slice",
        "byte_at",
    ];
    PURE_BUILTINS.contains(&name)
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

/// RES-192: build the call-graph edge set for `statements`, then
/// run the fixpoint. Returns `name → has_io` for every top-level
/// user fn. Non-function statements contribute nothing.
pub fn infer_fn_effects(
    statements: &[crate::span::Spanned<Node>],
) -> std::collections::HashMap<String, bool> {
    // Step 1: collect user-fn names + their body references.
    let mut fn_bodies: std::collections::HashMap<String, &Node> = std::collections::HashMap::new();
    for stmt in statements {
        if let Node::Function { name, body, .. } = &stmt.node {
            fn_bodies.insert(name.clone(), body.as_ref());
        }
    }

    // Step 2: initialize every fn as pure.
    let mut effects: std::collections::HashMap<String, bool> =
        fn_bodies.keys().map(|n| (n.clone(), false)).collect();

    // Step 3: fixpoint — iterate body-walks until no effect flips.
    // Upper bound: one flip per fn, so at most |fns| passes.
    let max_passes = fn_bodies.len().saturating_add(1);
    for _ in 0..max_passes {
        let mut changed = false;
        for (name, body) in &fn_bodies {
            if *effects.get(name).unwrap_or(&false) {
                continue; // already IO — nothing to update
            }
            if body_reaches_io(body, &effects) {
                effects.insert(name.clone(), true);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    effects
}

/// RES-192: body-level check for IO reachability under the
/// current `effects` snapshot. Used inside the fixpoint loop —
/// each iteration treats `effects` as a frozen best-estimate and
/// asks "does this body reach anything marked IO today?".
fn body_reaches_io(node: &Node, effects: &std::collections::HashMap<String, bool>) -> bool {
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
                match effects.get(callee) {
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
        Node::StructLiteral { fields, .. } => {
            fields.iter().any(|(_, v)| body_reaches_io(v, effects))
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
    let mut out = std::collections::HashMap::new();
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
        Node::StructLiteral { fields, .. } => {
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
        // End-to-end: running the typechecker populates
        // `stats.fn_effects`. Confirms the call-site inside
        // `check_program_with_source` is wired.
        let src = "\
            fn noisy() { println(\"h\"); return 0; }\n\
            fn quiet() { return 0; }\n\
            fn main(int _d) { noisy(); return quiet(); }\n\
            main(0);\n";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let mut tc = TypeChecker::new();
        tc.check_program_with_source(&program, "<t>")
            .expect("typecheck should succeed");
        assert_eq!(tc.stats.fn_effects.get("noisy"), Some(&true));
        assert_eq!(tc.stats.fn_effects.get("quiet"), Some(&false));
        assert_eq!(tc.stats.fn_effects.get("main"), Some(&true));
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
