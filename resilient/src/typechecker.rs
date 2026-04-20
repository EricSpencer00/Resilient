// Type checker module for Resilient language
use std::collections::HashMap;
use crate::{Node, Pattern};
use crate::span::Span;

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
        // RES-161a: outer name + whatever the inner pattern binds.
        Pattern::Bind(outer, inner) => {
            let mut bs = vec![outer.clone()];
            bs.extend(pattern_bindings(inner));
            bs
        }
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
    /// RES-192: inferred effect set per top-level user fn. `true`
    /// = reaches IO (direct or transitive call to an impure
    /// builtin, or to another IO fn, or to an unresolvable
    /// callee). `false` = pure. Populated by
    /// `infer_fn_effects` during `check_program_with_source`.
    pub fn_effects: std::collections::HashMap<String, bool>,
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
        | Node::TryExpression { span, .. } => *span,
        _ => Span::default(),
    }
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
        // RES-213: prefix/suffix tests + string repetition.
        env.set("starts_with".to_string(), Type::Function {
            params: vec![Type::String, Type::String],
            return_type: Box::new(Type::Bool),
        });
        env.set("ends_with".to_string(), Type::Function {
            params: vec![Type::String, Type::String],
            return_type: Box::new(Type::Bool),
        });
        env.set("repeat".to_string(), Type::Function {
            params: vec![Type::String, Type::Int],
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
            // RES-217: partial-proof warnings on by default.
            warn_unverified: true,
            // RES-217: populated by `check_program_with_source`.
            source_path: String::new(),
            // RES-189: populated during LetStatement handling.
            let_type_hints: Vec::new(),
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

                // RES-191: after regular type-checking, enforce the
                // `@pure` annotation. Collect the set of fn names
                // that are declared `@pure`, then re-walk each of
                // their bodies flagging any forbidden operation
                // (impure builtin, unannotated user-fn call, etc.).
                // Failures prepend the same file:line:col prefix as
                // above so users land on the violating site.
                check_program_purity(statements, source_path)?;

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
                                name, ty, PARAM_PRIMS.join(", ")
                            ));
                        }
                    }
                    if !RET_PRIMS.contains(&d.return_type.as_str()) {
                        return Err(format!(
                            "FFI: extern return type `{}` not supported in v1 (allowed: {})",
                            d.return_type, RET_PRIMS.join(", ")
                        ));
                    }
                }
                Ok(Type::Void)
            }
            
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
            
            Node::LetStatement { name, value, type_annot, span } => {
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
                    if !matches!(
                        value_type,
                        Type::Any | Type::Void | Type::Var(_)
                    ) {
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

                    // RES-159 + RES-160 + RES-161a: register every
                    // name the pattern binds (as the scrutinee's type)
                    // so guards and bodies can reference them. Rolled
                    // back after the arm so bindings don't leak.
                    // `pattern_bindings` returns [] for Wildcard/Literal,
                    // [n] for Identifier, [outer, ..inner] for Bind.
                    let binding_names = pattern_bindings(pattern);
                    let rollback_bindings: Vec<(String, Option<Type>)> = binding_names
                        .iter()
                        .map(|n| {
                            let prev = self.env.get(n);
                            self.env.set(n.clone(), scrutinee_type.clone());
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
                    // Roll back all pattern-binding entries.
                    for (n, prev) in rollback_bindings {
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
                            // RES-217: any unresolved verdict (timeout
                            // OR a genuine Z3 `Unknown`) is a partial
                            // proof. Emit the structured diagnostic
                            // with the specific assertion's source
                            // position; suppressed on
                            // `--no-warn-unverified`.
                            if verdict.is_none() && self.warn_unverified {
                                emit_partial_proof_warning(
                                    &self.source_path, clause,
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
    "println", "print", "input",
    // RES-147: monotonic clock.
    "clock_ms",
    // RES-150: seedable PRNG — nondeterministic from the caller's
    // point of view even though the seed pins it globally.
    "random_int", "random_float",
    // RES-143: disk I/O.
    "file_read", "file_write",
    // RES-151: env-var reads depend on process state outside
    // the fn.
    "env",
    // RES-138 / RES-141: retry-counter readers — observe runtime
    // state that isn't the fn's parameters.
    "live_retries", "live_total_retries", "live_total_exhaustions",
];

/// RES-191: top-level entry for the purity pass. Walks the
/// program's statement list once to collect `@pure` fn names
/// (their declarations include the `pure: bool` flag per the
/// ticket), then re-walks each declared-pure fn's body and
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
    let mut pure_fns: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for stmt in statements {
        if let Node::Function { name, pure: true, .. } = &stmt.node {
            pure_fns.insert(name.clone());
        }
    }

    // Second pass: check each pure fn's body.
    for stmt in statements {
        if let Node::Function { name, body, pure: true, .. } = &stmt.node
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
        Node::ReturnStatement { value: Some(value), .. } => {
            check_body_purity(value, fn_name, pure_fns)?;
        }
        Node::ReturnStatement { value: None, .. } => {}
        Node::IfStatement { condition, consequence, alternative, .. } => {
            check_body_purity(condition, fn_name, pure_fns)?;
            check_body_purity(consequence, fn_name, pure_fns)?;
            if let Some(a) = alternative {
                check_body_purity(a, fn_name, pure_fns)?;
            }
        }
        Node::WhileStatement { condition, body, .. } => {
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
        Node::LiveBlock { .. } => {
            // live-blocks retry on failure — that's observable,
            // non-pure behaviour by construction.
            return Err("contains a `live` block (retries are \
                        observable side effects)".to_string());
        }
        Node::InfixExpression { left, right, .. } => {
            check_body_purity(left, fn_name, pure_fns)?;
            check_body_purity(right, fn_name, pure_fns)?;
        }
        Node::PrefixExpression { right, .. } => {
            check_body_purity(right, fn_name, pure_fns)?;
        }
        Node::CallExpression { function, arguments, .. } => {
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
                    return Err(format!(
                        "calls impure builtin `{}`", callee
                    ));
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
                return Err(format!(
                    "calls unannotated fn `{}`", callee
                ));
            }
            // Indirect / method callee — can't resolve statically.
            // Conservatively reject so @pure is meaningful.
            check_body_purity(function, fn_name, pure_fns)?;
            return Err(
                "calls a non-identifier callee (method or computed); \
                 only bare-identifier calls to pure fns are allowed"
                    .to_string()
            );
        }
        Node::FieldAccess { target, .. } => {
            check_body_purity(target, fn_name, pure_fns)?;
        }
        Node::FieldAssignment { target, value, .. } => {
            check_body_purity(target, fn_name, pure_fns)?;
            check_body_purity(value, fn_name, pure_fns)?;
            // Mutating a field is observable — disallow.
            return Err(
                "mutates a struct field (field assignment is a side effect)".to_string()
            );
        }
        Node::Assignment { value, .. } => {
            check_body_purity(value, fn_name, pure_fns)?;
        }
        Node::IndexExpression { target, index, .. } => {
            check_body_purity(target, fn_name, pure_fns)?;
            check_body_purity(index, fn_name, pure_fns)?;
        }
        Node::IndexAssignment { target, index, value, .. } => {
            check_body_purity(target, fn_name, pure_fns)?;
            check_body_purity(index, fn_name, pure_fns)?;
            check_body_purity(value, fn_name, pure_fns)?;
            return Err(
                "mutates an array/map element (index assignment is a side effect)".to_string()
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
        Node::Match { scrutinee, arms, .. } => {
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
        "abs", "min", "max", "sqrt", "pow", "floor", "ceil",
        "to_float", "to_int",
        "sin", "cos", "tan", "ln", "log", "exp",
        // String/collection.
        "len", "push", "pop", "slice", "split", "trim", "contains",
        "to_upper", "to_lower", "replace", "format",
        "starts_with", "ends_with", "repeat",
        // Result helpers.
        "Ok", "Err", "is_ok", "is_err", "unwrap", "unwrap_err",
        // Map/Set/Bytes.
        "map_new", "map_insert", "map_get", "map_remove",
        "map_keys", "map_len",
        "set_new", "set_insert", "set_remove", "set_has",
        "set_len", "set_items",
        "bytes_len", "bytes_slice", "byte_at",
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
    let mut fn_bodies: std::collections::HashMap<String, &Node> =
        std::collections::HashMap::new();
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
fn body_reaches_io(
    node: &Node,
    effects: &std::collections::HashMap<String, bool>,
) -> bool {
    match node {
        Node::Block { stmts, .. } => stmts.iter().any(|s| body_reaches_io(s, effects)),
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            body_reaches_io(value, effects)
        }
        Node::ReturnStatement { value: Some(v), .. } => body_reaches_io(v, effects),
        Node::ReturnStatement { value: None, .. } => false,
        Node::IfStatement { condition, consequence, alternative, .. } => {
            body_reaches_io(condition, effects)
                || body_reaches_io(consequence, effects)
                || alternative
                    .as_ref()
                    .is_some_and(|a| body_reaches_io(a, effects))
        }
        Node::WhileStatement { condition, body, .. } => {
            body_reaches_io(condition, effects) || body_reaches_io(body, effects)
        }
        Node::ForInStatement { iterable, body, .. } => {
            body_reaches_io(iterable, effects) || body_reaches_io(body, effects)
        }
        Node::Assert { condition, .. } => body_reaches_io(condition, effects),
        Node::LiveBlock { body, invariants, .. } => {
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
        Node::CallExpression { function, arguments, .. } => {
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
        Node::IndexAssignment { target, index, value, .. } => {
            body_reaches_io(target, effects)
                || body_reaches_io(index, effects)
                || body_reaches_io(value, effects)
        }
        Node::ArrayLiteral { items, .. } => {
            items.iter().any(|i| body_reaches_io(i, effects))
        }
        Node::StructLiteral { fields, .. } => {
            fields.iter().any(|(_, v)| body_reaches_io(v, effects))
        }
        Node::Match { scrutinee, arms, .. } => {
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
        // Nested fn decls are rare but handled — recurse into
        // their body too. Today the parser doesn't emit these;
        // future closures will.
        Node::Function { body, .. } => body_reaches_io(body, effects),
        // Pure literals / identifier reads / etc.
        _ => false,
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
        let err = check_program_purity(&s, "<t>")
            .expect_err("unannotated user fn is rejected");
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
        check_program_purity(&s, "<t>")
            .expect("mutual recursion between two @pure fns is fine");
    }

    #[test]
    fn pure_fn_calling_live_block_is_rejected() {
        // `live` blocks retry on failure — observable from outside.
        let src = "@pure fn f(int x) { live { return x; } return 0; }\n";
        let s = stmts(src);
        let err = check_program_purity(&s, "<t>")
            .expect_err("live blocks are impure");
        assert!(err.contains("live"), "unexpected error: {err}");
    }

    #[test]
    fn unannotated_fn_is_not_checked_for_purity() {
        // Non-@pure fns are free to do anything; the checker must
        // leave them alone even if they'd violate purity.
        let src = "fn noisy() { println(\"hi\"); return 0; }\n";
        let s = stmts(src);
        check_program_purity(&s, "<t>")
            .expect("non-@pure fns bypass the purity checker");
    }

    // ---------- error message shape ----------

    #[test]
    fn error_mentions_fn_name_and_violating_site() {
        let src = "@pure fn noisy(int x) { println(\"hi\"); return x; }\n";
        let s = stmts(src);
        let err = check_program_purity(&s, "<t>").unwrap_err();
        assert!(err.contains("noisy"), "expected fn name in error: {err}");
        assert!(err.contains("println"), "expected callee name in error: {err}");
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
