// Type checker module for Resilient language
use std::collections::HashMap;
use crate::{Node, Pattern};

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
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
    /// Currently only the `unify` module's unit tests construct this
    /// variant; the `dead_code` allow goes away when RES-120 lands.
    #[allow(dead_code)]
    Var(u32),
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Type::Int => write!(f, "int"),
            Type::Float => write!(f, "float"),
            Type::String => write!(f, "string"),
            Type::Bool => write!(f, "bool"),
            Type::Bytes => write!(f, "bytes"),
            Type::Function { params, return_type } => {
                write!(f, "fn(")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", param)?;
                }
                write!(f, ") -> {}", return_type)
            },
            Type::Array => write!(f, "array"),
            Type::Result => write!(f, "Result"),
            Type::Struct(n) => write!(f, "{}", n),
            Type::Void => write!(f, "void"),
            Type::Any => write!(f, "any"),
            Type::Var(id) => write!(f, "?t{}", id),
        }
    }
}

/// RES-053: Two types are compatible if they're equal or if either is
/// Any. Used everywhere we need "same type, or we don't know yet."
fn compatible(a: &Type, b: &Type) -> bool {
    a == b || matches!(a, Type::Any) || matches!(b, Type::Any)
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
            branches
                .first()
                .map(pattern_bindings)
                .unwrap_or_default()
        }
    }
}

/// RES-160: if a pattern introduces exactly one identifier binding
/// (directly or through every branch of an or-pattern), return it;
/// otherwise `None`. Used to hook the match-arm scope.
fn pattern_single_binding(p: &Pattern) -> Option<String> {
    let bs = pattern_bindings(p);
    match bs.len() {
        1 => Some(bs.into_iter().next().unwrap()),
        _ => None,
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

/// RES-130: arithmetic operators (`+ - * / %`) require both
/// operands to be the same numeric type — no implicit int ↔ float
/// coercion. Any/Any fall through as Any for the inference-in-
/// progress path.
///
/// Returns the result type on success or a type-error diagnostic
/// pointing users at the explicit `to_float(x)` / `to_int(x)`
/// conversions when they mixed the two.
fn check_numeric_same_type(
    op: &str,
    left: &Type,
    right: &Type,
) -> Result<Type, String> {
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
        _ => Err(format!(
            "Cannot apply '{}' to {} and {}",
            op, left, right
        )),
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
        Node::PrefixExpression { operator, right, .. } if operator == "!" => {
            fold_const_bool(right, bindings).map(|b| !b)
        }
        Node::InfixExpression { left, operator, right, .. } => {
            match operator.as_str() {
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
            }
        }
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
    if let Node::InfixExpression { left, operator, right, .. } = cond
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
        Node::PrefixExpression { operator, right, .. } if operator == "-" => {
            fold_const_i64(right, bindings).map(|v| -v)
        }
        Node::InfixExpression { left, operator, right, .. } => {
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
/// failure. Without `--features z3`, returns all-`None` / `false`.
#[cfg(feature = "z3")]
fn z3_prove_with_cert(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    timeout_ms: u32,
) -> (Option<bool>, Option<String>, Option<String>, bool) {
    let (verdict, cert, cx, timed_out) =
        crate::verifier_z3::prove_with_timeout(expr, bindings, timeout_ms);
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

        // Math (single-arg — int/float passed as Any)
        env.set("abs".to_string(), fn_any_to_any());
        env.set("sqrt".to_string(), fn_any_to_any());
        env.set("floor".to_string(), fn_any_to_any());
        env.set("ceil".to_string(), fn_any_to_any());
        env.set("min".to_string(), fn_any_any_to_any());
        env.set("max".to_string(), fn_any_any_to_any());
        env.set("pow".to_string(), fn_any_any_to_any());

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
        env.set("log".to_string(), Type::Function {
            params: vec![Type::Float, Type::Float],
            return_type: Box::new(Type::Float),
        });

        // RES-147: monotonic ms-clock builtin. std-only.
        env.set("clock_ms".to_string(), Type::Function {
            params: vec![],
            return_type: Box::new(Type::Int),
        });

        // RES-150: seedable random builtins. std-only.
        env.set("random_int".to_string(), Type::Function {
            params: vec![Type::Int, Type::Int],
            return_type: Box::new(Type::Int),
        });
        env.set("random_float".to_string(), Type::Function {
            params: vec![],
            return_type: Box::new(Type::Float),
        });

        // len: any -> int
        env.set(
            "len".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Int),
            },
        );

        // Array builtins: any -> array / (array,int,int) -> array
        env.set("push".to_string(), Type::Function {
            params: vec![Type::Array, Type::Any],
            return_type: Box::new(Type::Array),
        });
        env.set("pop".to_string(), Type::Function {
            params: vec![Type::Array],
            return_type: Box::new(Type::Array),
        });
        env.set("slice".to_string(), Type::Function {
            params: vec![Type::Array, Type::Int, Type::Int],
            return_type: Box::new(Type::Array),
        });

        // String builtins
        env.set("split".to_string(), Type::Function {
            params: vec![Type::String, Type::String],
            return_type: Box::new(Type::Array),
        });
        env.set("trim".to_string(), Type::Function {
            params: vec![Type::String],
            return_type: Box::new(Type::String),
        });
        env.set("contains".to_string(), Type::Function {
            params: vec![Type::String, Type::String],
            return_type: Box::new(Type::Bool),
        });
        env.set("to_upper".to_string(), Type::Function {
            params: vec![Type::String],
            return_type: Box::new(Type::String),
        });
        env.set("to_lower".to_string(), Type::Function {
            params: vec![Type::String],
            return_type: Box::new(Type::String),
        });
        // RES-145: replace + format.
        env.set("replace".to_string(), Type::Function {
            params: vec![Type::String, Type::String, Type::String],
            return_type: Box::new(Type::String),
        });
        // `format`'s second argument is `Array<?>` — the prelude
        // `Type::Array` is untyped (no element-type parameter yet),
        // which fits the ticket's `Array<?>` signature.
        env.set("format".to_string(), Type::Function {
            params: vec![Type::String, Type::Array],
            return_type: Box::new(Type::String),
        });

        // RES-130: explicit int ↔ float conversions. These are the
        // only supported bridge between the two numeric types —
        // arithmetic and literal-match pattern equality both reject
        // implicit coercion (see `check_numeric_same_type`).
        env.set("to_float".to_string(), Type::Function {
            params: vec![Type::Any],
            return_type: Box::new(Type::Float),
        });
        env.set("to_int".to_string(), Type::Function {
            params: vec![Type::Any],
            return_type: Box::new(Type::Int),
        });

        // RES-138: current retry counter of the enclosing live block.
        env.set("live_retries".to_string(), Type::Function {
            params: vec![],
            return_type: Box::new(Type::Int),
        });

        // RES-141: process-wide live-block telemetry.
        env.set("live_total_retries".to_string(), Type::Function {
            params: vec![],
            return_type: Box::new(Type::Int),
        });
        env.set("live_total_exhaustions".to_string(), Type::Function {
            params: vec![],
            return_type: Box::new(Type::Int),
        });

        // RES-144: one-line stdin reader (std-only).
        env.set("input".to_string(), Type::Function {
            params: vec![Type::String],
            return_type: Box::new(Type::String),
        });

        // RES-143: file I/O builtins (std-only; the resilient-runtime
        // sibling crate has no builtins table so its no_std posture is
        // unaffected).
        env.set("file_read".to_string(), Type::Function {
            params: vec![Type::String],
            return_type: Box::new(Type::String),
        });
        env.set("file_write".to_string(), Type::Function {
            params: vec![Type::String, Type::String],
            return_type: Box::new(Type::Void),
        });

        // RES-151: read-only env-var accessor. `Result<String, String>`
        // — absence is a first-class outcome, not a runtime halt.
        env.set("env".to_string(), Type::Function {
            params: vec![Type::String],
            return_type: Box::new(Type::Result),
        });

        // RES-148: Map builtins. The typechecker doesn't (yet) carry
        // a dedicated `Type::Map<K, V>` constructor — following the
        // same permissive-Any convention as the Array / Result
        // builtins until G7 inference lands.
        env.set("map_new".to_string(), Type::Function {
            params: vec![],
            return_type: Box::new(Type::Any),
        });
        env.set("map_insert".to_string(), Type::Function {
            params: vec![Type::Any, Type::Any, Type::Any],
            return_type: Box::new(Type::Any),
        });
        env.set("map_get".to_string(), Type::Function {
            params: vec![Type::Any, Type::Any],
            return_type: Box::new(Type::Result),
        });
        env.set("map_remove".to_string(), Type::Function {
            params: vec![Type::Any, Type::Any],
            return_type: Box::new(Type::Any),
        });
        env.set("map_keys".to_string(), Type::Function {
            params: vec![Type::Any],
            return_type: Box::new(Type::Array),
        });
        env.set("map_len".to_string(), Type::Function {
            params: vec![Type::Any],
            return_type: Box::new(Type::Int),
        });

        // RES-149: Set builtins. Same permissive-Any convention as
        // Map — no dedicated `Type::Set<T>` until inference lands.
        env.set("set_new".to_string(), Type::Function {
            params: vec![],
            return_type: Box::new(Type::Any),
        });
        env.set("set_insert".to_string(), Type::Function {
            params: vec![Type::Any, Type::Any],
            return_type: Box::new(Type::Any),
        });
        env.set("set_remove".to_string(), Type::Function {
            params: vec![Type::Any, Type::Any],
            return_type: Box::new(Type::Any),
        });
        env.set("set_has".to_string(), Type::Function {
            params: vec![Type::Any, Type::Any],
            return_type: Box::new(Type::Bool),
        });
        env.set("set_len".to_string(), Type::Function {
            params: vec![Type::Any],
            return_type: Box::new(Type::Int),
        });
        env.set("set_items".to_string(), Type::Function {
            params: vec![Type::Any],
            return_type: Box::new(Type::Array),
        });

        // RES-152: Bytes builtins. `bytes_slice` returns new Bytes;
        // `byte_at` returns Int — the language has no `u8` yet.
        env.set("bytes_len".to_string(), Type::Function {
            params: vec![Type::Bytes],
            return_type: Box::new(Type::Int),
        });
        env.set("bytes_slice".to_string(), Type::Function {
            params: vec![Type::Bytes, Type::Int, Type::Int],
            return_type: Box::new(Type::Bytes),
        });
        env.set("byte_at".to_string(), Type::Function {
            params: vec![Type::Bytes, Type::Int],
            return_type: Box::new(Type::Int),
        });

        // Result builtins
        env.set("Ok".to_string(), fn_any_to_result());
        env.set("Err".to_string(), fn_any_to_result());
        env.set("is_ok".to_string(), fn_result_to_bool());
        env.set("is_err".to_string(), fn_result_to_bool());
        env.set("unwrap".to_string(), fn_result_to_any());
        env.set("unwrap_err".to_string(), fn_result_to_any());

        TypeChecker {
            env,
            contract_table: HashMap::new(),
            const_bindings: HashMap::new(),
            stats: VerificationStats::default(),
            certificates: Vec::new(),
            struct_fields: HashMap::new(),
            type_aliases: HashMap::new(),
            // RES-137: ticket's default is 5 seconds per query.
            verifier_timeout_ms: 5000,
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
                            ..
                        } => {
                            self.contract_table.insert(
                                name.clone(),
                                ContractInfo {
                                    parameters: parameters.clone(),
                                    requires: requires.clone(),
                                    ensures: ensures.clone(),
                                },
                            );
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
                                source_path,
                                stmt.span.start.line,
                                stmt.span.start.column,
                                e
                            )
                        }
                    })?;
                }
                Ok(result_type)
            }
            _ => Err("Expected program node".to_string()),
        }
    }
    
    pub fn check_node(&mut self, node: &Node) -> Result<Type, String> {
        match node {
            Node::Program(_statements) => self.check_program(node),
            // RES-073: `use` is resolved away before typecheck. Treat
            // leftovers as void (no-op) for safety.
            Node::Use { .. } => Ok(Type::Void),
            
            Node::Function { name, parameters, body, requires, ensures, return_type: declared_rt, .. } => {
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
                        let (v, cert, cx, timed_out) =
                            z3_prove_with_cert(clause, &no_bindings, self.verifier_timeout_ms);
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

                // Check function body
                let body_type = self.check_node(body)?;

                // Restore const_bindings to its pre-body state.
                for (aname, prev) in pushed_assumptions.into_iter().rev() {
                    match prev {
                        Some(v) => { self.const_bindings.insert(aname, v); }
                        None => { self.const_bindings.remove(&aname); }
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
            },
            
            Node::LiveBlock { body, .. } => {
                // Live blocks preserve the type of their body
                self.check_node(body)
            },

            // RES-142: duration literals are a parser-internal node
            // that only appear inside a `live ... within <duration>`
            // clause; the parser stores them on `LiveBlock::timeout`
            // rather than emitting them as general expressions. If
            // one reaches the typechecker, treat it as `Int`
            // (nanosecond count) — defensive; should never fire in
            // well-formed programs.
            Node::DurationLiteral { .. } => Ok(Type::Int),
            
            Node::Assert { condition, message, .. } => {
                // Condition must be a boolean expression
                let condition_type = self.check_node(condition)?;
                if condition_type != Type::Bool && condition_type != Type::Any {
                    return Err(format!("Assert condition must be a boolean, got {}", condition_type));
                }
                
                // Message, if present, should be a string
                if let Some(msg) = message {
                    let msg_type = self.check_node(msg)?;
                    if msg_type != Type::String && msg_type != Type::Any {
                        return Err(format!("Assert message must be a string, got {}", msg_type));
                    }
                }
                
                Ok(Type::Void)
            },
            
            Node::Block { stmts: statements, .. } => {
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
            },
            
            Node::LetStatement { name, value, type_annot, .. } => {
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
            },

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
                            return Err(format!(
                                "Struct {} has no field `{}`",
                                struct_name, pf
                            ));
                        }
                    }
                    // Exhaustiveness check when `..` is not used.
                    if !has_rest {
                        let mut missing: Vec<&str> = declared_fields
                            .iter()
                            .filter(|(fname, _)| {
                                !fields.iter().any(|(pf, _)| pf == fname)
                            })
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
                            dfs.iter().find(|(fn_, _)| fn_ == field_name).map(|(_, t)| t.clone())
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
            },

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
            },

            // RES-149: set literal. Walk each item to catch nested
            // type errors; return `Type::Any` for now — same posture
            // as `MapLiteral` until `Type::Set<T>` shows up.
            Node::SetLiteral { items, .. } => {
                for item in items {
                    let _ = self.check_node(item)?;
                }
                Ok(Type::Any)
            },

            Node::TryExpression { expr: inner, .. } => {
                let inner_type = self.check_node(inner)?;
                // `?` expects a Result and unwraps to Any at MVP (we
                // don't track Ok's payload type yet).
                if !compatible(&inner_type, &Type::Result) {
                    return Err(format!(
                        "? operator expects a Result, got {}",
                        inner_type
                    ));
                }
                Ok(Type::Any)
            },

            Node::FunctionLiteral { parameters, body, .. } => {
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
            },

            Node::Match { scrutinee, arms, .. } => {
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

                    // RES-159 + RES-160: if the arm's pattern binds
                    // an identifier — either directly or through
                    // every branch of an Or — register that binding
                    // (as the scrutinee's type) so guards and bodies
                    // can reference it. Rolled back after the arm so
                    // it doesn't leak. Mirrors the interpreter's
                    // scoping behaviour.
                    let binding_name = pattern_single_binding(pattern);
                    let rollback_ident: Option<(String, Option<Type>)> =
                        if let Some(n) = binding_name {
                            let prev = self.env.get(&n);
                            self.env.set(n.clone(), scrutinee_type.clone());
                            Some((n, prev))
                        } else {
                            None
                        };

                    if let Some(g) = guard {
                        // RES-159: guards must be boolean-ish. Accept
                        // Bool / Any so existing permissive inference
                        // stays compatible.
                        let gt = self.check_node(g)?;
                        if gt != Type::Bool && gt != Type::Any {
                            // Restore env before propagating.
                            if let Some((n, prev)) = &rollback_ident {
                                match prev {
                                    Some(t) => self.env.set(n.clone(), t.clone()),
                                    None => { self.env.remove(n); }
                                }
                            }
                            return Err(format!(
                                "Match arm guard must be a boolean, got {}",
                                gt
                            ));
                        }
                    }
                    let body_res = self.check_node(body);
                    // Roll back the pattern-binding entry.
                    if let Some((n, prev)) = rollback_ident {
                        match prev {
                            Some(t) => self.env.set(n, t),
                            None => { self.env.remove(&n); }
                        }
                    }
                    let _ = body_res?;
                }

                // RES-054 + RES-159 + RES-160: exhaustiveness check.
                // An arm is "covering" when it's unguarded AND its
                // pattern contains at least one wildcard / identifier
                // branch — `pattern_is_default` recurses through
                // or-patterns so `_ | x` and `0 | _` both count.
                let has_default = arms.iter().any(|(p, guard, _)| {
                    guard.is_none() && pattern_is_default(p)
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
                        other => {
                            return Err(format!(
                                "Non-exhaustive match on {}: add a wildcard `_` or identifier arm to handle unmatched values",
                                other
                            ));
                        }
                    }
                }

                Ok(Type::Any)
            },

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
            },

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
            },

            Node::FieldAssignment { target, field, value, .. } => {
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
                    let avail: Vec<&str> =
                        declared.iter().map(|(n, _)| n.as_str()).collect();
                    return Err(format!(
                        "struct `{}` has no field `{}`; available fields: {}",
                        sname,
                        field,
                        avail.join(", ")
                    ));
                }
                Ok(Type::Void)
            },

            Node::IndexExpression { target, index, .. } => {
                let _ = self.check_node(target)?;
                let _ = self.check_node(index)?;
                // Element type not tracked at MVP.
                Ok(Type::Any)
            },

            Node::IndexAssignment { target, index, value, .. } => {
                let _ = self.check_node(target)?;
                let _ = self.check_node(index)?;
                let _ = self.check_node(value)?;
                Ok(Type::Void)
            },

            Node::ForInStatement { iterable, body, .. } => {
                let _ = self.check_node(iterable)?;
                let _ = self.check_node(body)?;
                Ok(Type::Void)
            },

            Node::WhileStatement { condition, body, .. } => {
                let _ = self.check_node(condition)?;
                let _ = self.check_node(body)?;
                Ok(Type::Void)
            },

            Node::StaticLet { name, value, .. } => {
                let value_type = self.check_node(value)?;
                self.env.set(name.clone(), value_type);
                // RES-063: static lets are mutable across calls, so
                // they're never safe to treat as compile-time constants
                // for verification.
                self.const_bindings.remove(name);
                Ok(Type::Void)
            },

            Node::Assignment { name, value, .. } => {
                let _ = self.check_node(value)?;
                // RES-063: any reassignment kills const-tracking. We
                // could try to re-track if RHS is foldable, but
                // mid-function mutation is rare and the conservative
                // choice keeps the verifier sound.
                self.const_bindings.remove(name);
                Ok(Type::Void)
            },
            
            Node::ReturnStatement { value, .. } => {
                // Bare `return;` has type Void; otherwise pass through
                // the type of the returned value.
                match value {
                    Some(expr) => self.check_node(expr),
                    None => Ok(Type::Void),
                }
            },
            
            Node::IfStatement { condition, consequence, alternative, .. } => {
                let condition_type = self.check_node(condition)?;
                if condition_type != Type::Bool && condition_type != Type::Any {
                    return Err(format!("If condition must be a boolean, got {}", condition_type));
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
                        Some(v) => { self.const_bindings.insert(name, v); }
                        None => { self.const_bindings.remove(&name); }
                    }
                }

                if let Some(alt) = alternative {
                    let alternative_type = self.check_node(alt)?;

                    // Both branches should have compatible types
                    if consequence_type != alternative_type &&
                       consequence_type != Type::Any &&
                       alternative_type != Type::Any {
                        return Err(format!("If branches have incompatible types: {} and {}",
                                          consequence_type, alternative_type));
                    }
                }

                Ok(consequence_type)
            },
            
            Node::ExpressionStatement { expr, .. } => {
                self.check_node(expr)
            },
            
            Node::Identifier { name, span } => {
                // RES-078: identifier span lets us tell users where
                // exactly the undefined reference lives. Skip the
                // prefix when the span looks default (synthetic).
                match self.env.get(name) {
                    Some(typ) => Ok(typ),
                    None => {
                        if span.start.line == 0 {
                            Err(format!("Undefined variable: {}", name))
                        } else {
                            Err(format!(
                                "Undefined variable '{}' at {}:{}",
                                name, span.start.line, span.start.column
                            ))
                        }
                    }
                }
            },
            
            Node::IntegerLiteral { .. } => Ok(Type::Int),
            Node::FloatLiteral { .. } => Ok(Type::Float),
            Node::StringLiteral { .. } => Ok(Type::String),
            Node::BytesLiteral { .. } => Ok(Type::Bytes),
            Node::BooleanLiteral { .. } => Ok(Type::Bool),
            
            Node::PrefixExpression { operator, right, .. } => {
                let right_type = self.check_node(right)?;
                
                match operator.as_str() {
                    "!" => {
                        if right_type != Type::Bool && right_type != Type::Any {
                            return Err(format!("Cannot apply '!' to {}", right_type));
                        }
                        Ok(Type::Bool)
                    },
                    "-" => {
                        if right_type != Type::Int && right_type != Type::Float && right_type != Type::Any {
                            return Err(format!("Cannot apply '-' to {}", right_type));
                        }
                        Ok(right_type)
                    },
                    _ => Err(format!("Unknown prefix operator: {}", operator)),
                }
            },
            
            Node::InfixExpression { left, operator, right, .. } => {
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
                        // Bitwise operators are int-only.
                        if compatible(&left_type, &Type::Int)
                            && compatible(&right_type, &Type::Int)
                        {
                            Ok(Type::Int)
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
                            Err(format!(
                                "Cannot compare {} and {}",
                                left_type, right_type
                            ))
                        }
                    }
                    _ => Err(format!("Unknown infix operator: {}", operator)),
                }
            },
            
            Node::CallExpression { function, arguments, .. } => {
                let func_type = self.check_node(function)?;

                // RES-061 + RES-063: if the callee is a known top-level
                // fn with contracts, fold each requires clause with the
                // call's arguments substituted for parameters. Arguments
                // can be literal expressions OR identifiers that resolve
                // to a constant via const_bindings.
                if let Node::Identifier { name: callee_name, .. } = function.as_ref()
                    && let Some(info) = self.contract_table.get(callee_name).cloned()
                {
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
                            let (v, cert, cx, timed_out) =
                                z3_prove_with_cert(clause, &bindings, self.verifier_timeout_ms);
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
                                *self.stats.per_fn_discharged
                                    .entry(callee_name.clone()).or_insert(0) += 1;
                            }
                            None => {
                                self.stats.requires_left_for_runtime += 1;
                                *self.stats.per_fn_runtime
                                    .entry(callee_name.clone()).or_insert(0) += 1;
                            }
                        }
                    }
                }

                match func_type {
                    Type::Function { params, return_type } => {
                        // Check argument count
                        if arguments.len() != params.len() {
                            return Err(format!("Expected {} arguments, got {}",
                                              params.len(), arguments.len()));
                        }

                        // Check each argument type
                        for (i, (arg, param_type)) in arguments.iter().zip(params.iter()).enumerate() {
                            let arg_type = self.check_node(arg)?;
                            if arg_type != *param_type && *param_type != Type::Any && arg_type != Type::Any {
                                return Err(format!("Type mismatch in argument {}: expected {}, got {}",
                                                  i + 1, param_type, arg_type));
                            }
                        }

                        Ok(*return_type)
                    },
                    Type::Any => {
                        Ok(Type::Any)
                    },
                    _ => Err(format!("Cannot call non-function type: {}", func_type)),
                }
            },
        }
    }
    
    fn parse_type_name(&self, name: &str) -> Result<Type, String> {
        self.parse_type_name_inner(name, &mut Vec::new())
    }

    /// RES-128: alias-aware parse with cycle detection. `seen`
    /// tracks the alias names we've already expanded on the
    /// current walk — re-entering any of them means the user
    /// wrote a loop (`type A = B; type B = A;`), which we surface
    /// as a diagnostic instead of looping forever or stack-
    /// overflowing.
    fn parse_type_name_inner(&self, name: &str, seen: &mut Vec<String>) -> Result<Type, String> {
        match name {
            "int" => Ok(Type::Int),
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
                    return Err(format!(
                        "type alias cycle: {}",
                        chain.join(" -> ")
                    ));
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
