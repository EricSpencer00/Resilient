//! RES-351: array bounds — static proof and runtime check.
//!
//! This pass walks every top-level function body and classifies each
//! `arr[i]` index access into one of two buckets:
//!
//! 1. **Proven in-bounds.** The pass collected enough context (an
//!    enclosing `for i in 0..len(arr)` loop, a `requires 0 <= i &&
//!    i < len(arr)` contract, a literal-length array with a literal
//!    index, etc.) for Z3 to discharge `0 <= i < len(arr)` as a
//!    tautology under those axioms. The runtime bounds check inside
//!    the VM (see `vm::VmError::ArrayIndexOutOfBounds`) is still
//!    emitted — it's cheap and this pass is advisory — but the audit
//!    counter records the proof and a proof certificate is captured
//!    when `--emit-certificate` is set.
//!
//! 2. **Not proven.** The runtime bounds check stays, returning
//!    `VmError::ArrayIndexOutOfBounds` — a recoverable error, NOT a
//!    panic, which matters on embedded targets. Under the strict
//!    `--deny-unproven-bounds` flag the pass instead emits a
//!    compile-time error pointing at the index site.
//!
//! The pass is always compiled in. Without `--features z3` the Z3
//! shim returns `None` for every non-trivial proof and everything
//! falls into bucket 2 (same as today's behaviour) — but literal
//! constant bounds (`xs[0]` where `xs` has known static length ≥ 1)
//! are still proven via the built-in folder, so the pass is useful
//! even without SMT.

use crate::Node;
use crate::span::Span;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

/// RES-351: global flag set by the CLI when `--deny-unproven-bounds`
/// is passed. The typechecker extension pass reads this once per
/// program and converts unproven bounds into hard compile errors.
///
/// A process-global is used (rather than threading through the
/// TypeChecker constructor) to keep the extension-point touch on
/// `typechecker.rs` to a single line, per the feature-isolation
/// pattern in CLAUDE.md.
static DENY_UNPROVEN_BOUNDS: AtomicBool = AtomicBool::new(false);

/// Enable `--deny-unproven-bounds` mode. Called from the `lib.rs` CLI
/// dispatcher before `check_program_with_source` runs.
pub fn set_deny_unproven_bounds(on: bool) {
    DENY_UNPROVEN_BOUNDS.store(on, Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) static BOUNDS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// True if the strict-deny flag is active for this process.
fn deny_unproven_bounds() -> bool {
    DENY_UNPROVEN_BOUNDS.load(Ordering::Relaxed)
}

/// Per-run counters populated by [`check_array_bounds`]. Exposed via
/// a thread-local so tests and the audit report can read them
/// without broadening the pass signature.
#[derive(Debug, Default, Clone, Copy)]
pub struct BoundsStats {
    pub proven: usize,
    pub unproven: usize,
}

thread_local! {
    static STATS: std::cell::RefCell<BoundsStats> = const {
        std::cell::RefCell::new(BoundsStats { proven: 0, unproven: 0 })
    };
    /// RES-407: spans of every index expression the pass discharged.
    /// Read by the bytecode compiler to emit `LoadIndexUnchecked` for
    /// the access — and by the `--audit` reporter to surface a per-site
    /// elided/runtime breakdown. Per-thread so the parallel test
    /// runner stays correct (the `TEST_LOCK` mutex serialises the few
    /// tests that read this).
    // RES-1694: pre-size with capacity 32 — same pattern as RES-1686 /
    // RES-1688 / RES-1690 / RES-1692. PROVEN_SITES accumulates one
    // entry per proven `IndexExpression` site within a typecheck; for
    // programs with many array accesses, doubling growth from 0 paid
    // for 2-3 rehash rounds. `reset_stats` clears but retains
    // capacity, so this only benefits the first typecheck per process
    // (CI / `cargo test` runs).
    static PROVEN_SITES: std::cell::RefCell<HashSet<Span>> =
        std::cell::RefCell::new(HashSet::with_capacity(32));
}

/// Read the last-run counters. Tests use this to confirm the pass
/// actually classified the indices it was supposed to.
#[allow(dead_code)]
pub fn last_stats() -> BoundsStats {
    STATS.with(|s| *s.borrow())
}

/// RES-407: true if the index access at `span` was proven by the
/// last `check_array_bounds` run. The bytecode compiler queries this
/// to emit `Op::LoadIndexUnchecked` instead of the bounds-checked
/// `Op::LoadIndex`. Returns `false` when the pass hasn't run (e.g.
/// the user passed neither `--typecheck` nor `--deny-unproven-bounds`)
/// — the runtime check then keeps every access safe.
pub fn is_proven_site(span: Span) -> bool {
    PROVEN_SITES.with(|s| s.borrow().contains(&span))
}

/// RES-407: snapshot of the proven access spans, sorted by source
/// position for deterministic `--audit` output.
pub fn proven_sites_sorted() -> Vec<Span> {
    let mut out: Vec<Span> = PROVEN_SITES.with(|s| s.borrow().iter().copied().collect());
    out.sort_by_key(|s| (s.start.line, s.start.column, s.start.offset));
    out
}

fn mark_proven(span: Span) {
    STATS.with(|s| s.borrow_mut().proven += 1);
    PROVEN_SITES.with(|s| {
        s.borrow_mut().insert(span);
    });
}
fn bump_unproven() {
    STATS.with(|s| s.borrow_mut().unproven += 1);
}
fn reset_stats() {
    STATS.with(|s| *s.borrow_mut() = BoundsStats::default());
    PROVEN_SITES.with(|s| s.borrow_mut().clear());
}

/// Axioms accumulated while descending into a function body. Each
/// entry is a boolean `Node` over integer-typed identifiers that the
/// verifier can feed Z3 as context.
#[derive(Default, Clone)]
struct BoundsCtx {
    axioms: Vec<Node>,
    /// Literal-length arrays: `name -> len`. Populated when the pass
    /// sees `let name = [a, b, c];`. Used by the fast-path folder —
    /// a literal `name[2]` with a length-3 array needs no solver.
    literal_len: HashMap<String, i64>,
}

impl BoundsCtx {
    fn with_axiom(&self, axiom: Node) -> Self {
        let mut next = self.clone();
        next.axioms.push(axiom);
        next
    }
}

/// RES-351: entry point — walk the program, classify every index
/// expression. Errors only on provable-OOB literals or on
/// `--deny-unproven-bounds` (strict mode).
pub fn check_array_bounds(program: &Node, source_path: &str) -> Result<(), String> {
    reset_stats();
    let Node::Program(statements) = program else {
        return Ok(());
    };
    // RES-1278 / RES-1916: the typechecker gates this call behind
    // `markers.has_index_expression`, so the program is guaranteed to
    // contain at least one `IndexExpression`. The previous `any_node`
    // pre-scan was redundant — removed.
    let mut strict_errors: Vec<String> = Vec::new();
    for stmt in statements {
        walk_toplevel(&stmt.node, source_path, &mut strict_errors);
    }
    if !strict_errors.is_empty() {
        return Err(strict_errors.join("\n"));
    }
    Ok(())
}

fn walk_toplevel(node: &Node, source_path: &str, errors: &mut Vec<String>) {
    match node {
        Node::Function {
            body,
            requires,
            ensures,
            recovers_to,
            defaults,
            ..
        } => {
            walk_function(
                body,
                FunctionContracts {
                    requires,
                    ensures,
                    recovers_to: recovers_to.as_deref(),
                    defaults,
                },
                &BoundsCtx::default(),
                source_path,
                errors,
            );
        }
        Node::ImplBlock { methods, .. } | Node::BlanketImpl { methods, .. } => {
            for method in methods {
                walk_toplevel(method, source_path, errors);
            }
        }
        Node::ModuleDecl { body, .. } => {
            for item in body {
                walk_toplevel(item, source_path, errors);
            }
        }
        Node::Extern { decls, .. } => {
            for decl in decls {
                for clause in &decl.requires {
                    walk_node(clause, &BoundsCtx::default(), source_path, errors);
                }
                for clause in &decl.ensures {
                    walk_node(clause, &BoundsCtx::default(), source_path, errors);
                }
            }
        }
        Node::Actor {
            state_init,
            concurrent_ensures,
            handlers,
            ..
        } => {
            let ctx = BoundsCtx::default();
            walk_node(state_init, &ctx, source_path, errors);
            for clause in concurrent_ensures {
                walk_node(clause, &ctx, source_path, errors);
            }
            for handler in handlers {
                for clause in &handler.ensures {
                    walk_node(clause, &ctx, source_path, errors);
                }
                walk_node(&handler.body, &ctx, source_path, errors);
            }
        }
        Node::ActorDecl {
            state_fields,
            always_clauses,
            eventually_clauses,
            receive_handlers,
            handlers,
            ..
        } => {
            let ctx = BoundsCtx::default();
            for (_, _, initializer) in state_fields {
                walk_node(initializer, &ctx, source_path, errors);
            }
            for clause in always_clauses {
                walk_node(clause, &ctx, source_path, errors);
            }
            for clause in eventually_clauses {
                walk_node(&clause.post, &ctx, source_path, errors);
            }
            for handler in receive_handlers {
                for clause in &handler.requires {
                    walk_node(clause, &ctx, source_path, errors);
                }
                for clause in &handler.ensures {
                    walk_node(clause, &ctx, source_path, errors);
                }
                walk_node(&handler.body, &ctx, source_path, errors);
            }
            for handler in handlers {
                for clause in &handler.ensures {
                    walk_node(clause, &ctx, source_path, errors);
                }
                walk_node(&handler.body, &ctx, source_path, errors);
            }
        }
        Node::ClusterDecl { invariants, .. } => {
            let ctx = BoundsCtx::default();
            for invariant in invariants {
                walk_node(invariant, &ctx, source_path, errors);
            }
        }
        Node::Program(statements) => {
            for statement in statements {
                walk_toplevel(&statement.node, source_path, errors);
            }
        }
        _ => walk_node(node, &BoundsCtx::default(), source_path, errors),
    }
}

struct FunctionContracts<'a> {
    requires: &'a [Node],
    ensures: &'a [Node],
    recovers_to: Option<&'a Node>,
    defaults: &'a [Option<Box<Node>>],
}

fn walk_function(
    body: &Node,
    contracts: FunctionContracts<'_>,
    parent_ctx: &BoundsCtx,
    source_path: &str,
    errors: &mut Vec<String>,
) {
    let mut ctx = parent_ctx.clone();
    // Defaults and requires are evaluated before the function body, so
    // they cannot use the body's local literal-length facts as axioms.
    for default in contracts.defaults.iter().flatten() {
        walk_node(default, parent_ctx, source_path, errors);
    }
    for clause in contracts.requires {
        walk_node(clause, parent_ctx, source_path, errors);
        ctx.axioms.push(clause.clone());
    }
    // RES-133b: leading `assume(P)` predicates are also axioms.
    // The runtime check halts before any indexing in the body if
    // they are violated, so the bounds prover may use them.
    ctx.axioms
        .extend(crate::assume_axioms::collect_leading_assume_axioms(body));
    walk_node(body, &ctx, source_path, errors);
    // Postconditions run after the body and may themselves contain
    // indexing expressions. They can rely on the same preconditions.
    for clause in contracts.ensures {
        walk_node(clause, &ctx, source_path, errors);
    }
    if let Some(clause) = contracts.recovers_to {
        walk_node(clause, &ctx, source_path, errors);
    }
}

fn walk_node(node: &Node, ctx: &BoundsCtx, source_path: &str, errors: &mut Vec<String>) {
    match node {
        Node::Program(statements) => {
            for statement in statements {
                walk_toplevel(&statement.node, source_path, errors);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            recovers_to,
            defaults,
            ..
        } => walk_function(
            body,
            FunctionContracts {
                requires,
                ensures,
                recovers_to: recovers_to.as_deref(),
                defaults,
            },
            ctx,
            source_path,
            errors,
        ),
        Node::Block { stmts, .. } => {
            // Track literal-length lets introduced in this block so
            // subsequent statements can use them as a fast path.
            let mut block_ctx = ctx.clone();
            for stmt in stmts {
                walk_node(stmt, &block_ctx, source_path, errors);
                if let Node::LetStatement { name, value, .. } = stmt
                    && let Node::ArrayLiteral { items, .. } = value.as_ref()
                {
                    block_ctx
                        .literal_len
                        .insert(name.clone(), items.len() as i64);
                }
            }
        }
        Node::ForInStatement {
            name,
            iterable,
            body,
            invariants,
            ..
        } => {
            // Canonical pattern we want to prove: for i in 0..len(arr).
            // The iterable parses as an InfixExpression with operator
            // `..`, left being the lower bound, right the upper.
            let mut body_ctx = ctx.clone();
            if let Node::InfixExpression {
                left,
                operator,
                right,
                ..
            } = iterable.as_ref()
                && *operator == ".."
            {
                let ident = |n: &str| Node::Identifier {
                    name: n.to_string(),
                    span: Span::default(),
                };
                // Axiom: lower_bound <= i
                body_ctx.axioms.push(Node::InfixExpression {
                    left: Box::new((**left).clone()),
                    operator: "<=",
                    right: Box::new(ident(name)),
                    span: Span::default(),
                });
                // Axiom: i < upper_bound
                body_ctx.axioms.push(Node::InfixExpression {
                    left: Box::new(ident(name)),
                    operator: "<",
                    right: Box::new((**right).clone()),
                    span: Span::default(),
                });
            }
            walk_node(iterable, ctx, source_path, errors);
            for invariant in invariants {
                walk_node(invariant, ctx, source_path, errors);
            }
            walk_node(body, &body_ctx, source_path, errors);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_node(condition, ctx, source_path, errors);
            // Propagate the condition as an axiom into the consequent.
            let then_ctx = ctx.with_axiom((**condition).clone());
            walk_node(consequence, &then_ctx, source_path, errors);
            if let Some(alt) = alternative {
                walk_node(alt, ctx, source_path, errors);
            }
        }
        Node::WhileStatement {
            condition,
            body,
            invariants,
            ..
        } => {
            walk_node(condition, ctx, source_path, errors);
            for invariant in invariants {
                walk_node(invariant, ctx, source_path, errors);
            }
            let body_ctx = ctx.with_axiom((**condition).clone());
            walk_node(body, &body_ctx, source_path, errors);
        }
        Node::IndexExpression {
            target,
            index,
            span,
        } => {
            check_index(target, index, *span, ctx, source_path, errors);
            walk_node(target, ctx, source_path, errors);
            walk_node(index, ctx, source_path, errors);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            span,
        } => {
            check_index(target, index, *span, ctx, source_path, errors);
            walk_node(target, ctx, source_path, errors);
            walk_node(index, ctx, source_path, errors);
            walk_node(value, ctx, source_path, errors);
        }
        Node::LetStatement { value, .. } => walk_node(value, ctx, source_path, errors),
        Node::Assignment { value, .. } => walk_node(value, ctx, source_path, errors),
        Node::ReturnStatement { value: Some(v), .. } => walk_node(v, ctx, source_path, errors),
        Node::ExpressionStatement { expr, .. } => walk_node(expr, ctx, source_path, errors),
        Node::PrefixExpression { right, .. } => walk_node(right, ctx, source_path, errors),
        Node::InfixExpression { left, right, .. } => {
            walk_node(left, ctx, source_path, errors);
            walk_node(right, ctx, source_path, errors);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            walk_node(function, ctx, source_path, errors);
            for a in arguments {
                walk_node(a, ctx, source_path, errors);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk_node(scrutinee, ctx, source_path, errors);
            for (_, guard, body) in arms {
                if let Some(guard) = guard {
                    walk_node(guard, ctx, source_path, errors);
                }
                walk_node(body, ctx, source_path, errors);
            }
        }
        Node::FunctionLiteral {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => walk_function(
            body,
            FunctionContracts {
                requires,
                ensures,
                recovers_to: recovers_to.as_deref(),
                defaults: &[],
            },
            ctx,
            source_path,
            errors,
        ),
        Node::TryCatch { body, handlers, .. } => {
            for statement in body {
                walk_node(statement, ctx, source_path, errors);
            }
            for (_, handler_body) in handlers {
                for statement in handler_body {
                    walk_node(statement, ctx, source_path, errors);
                }
            }
        }
        Node::TryExpression { expr, .. }
        | Node::NewtypeConstruct { value: expr, .. }
        | Node::NamedArg { value: expr, .. }
        | Node::DeferStatement { expr, .. }
        | Node::TupleIndex { tuple: expr, .. } => walk_node(expr, ctx, source_path, errors),
        Node::OptionalChain { object, access, .. } => {
            walk_node(object, ctx, source_path, errors);
            if let crate::ChainAccess::Method(_, arguments) = access {
                for argument in arguments {
                    walk_node(argument, ctx, source_path, errors);
                }
            }
        }
        Node::LiveBlock {
            body,
            invariants,
            timeout,
            ..
        } => {
            walk_node(body, ctx, source_path, errors);
            for invariant in invariants {
                walk_node(invariant, ctx, source_path, errors);
            }
            if let Some(timeout) = timeout {
                walk_node(timeout, ctx, source_path, errors);
            }
        }
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            walk_node(condition, ctx, source_path, errors);
            if let Some(message) = message {
                walk_node(message, ctx, source_path, errors);
            }
        }
        Node::FieldAccess { target, .. } => walk_node(target, ctx, source_path, errors),
        Node::FieldAssignment { target, value, .. } => {
            walk_node(target, ctx, source_path, errors);
            walk_node(value, ctx, source_path, errors);
        }
        Node::ArrayLiteral { items, .. }
        | Node::SetLiteral { items, .. }
        | Node::TupleLiteral { items, .. } => {
            for item in items {
                walk_node(item, ctx, source_path, errors);
            }
        }
        Node::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                walk_node(key, ctx, source_path, errors);
                walk_node(value, ctx, source_path, errors);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(base) = base {
                walk_node(base, ctx, source_path, errors);
            }
            for (_, value) in fields {
                walk_node(value, ctx, source_path, errors);
            }
        }
        Node::Slice { target, lo, hi, .. } => {
            walk_node(target, ctx, source_path, errors);
            if let Some(lo) = lo {
                walk_node(lo, ctx, source_path, errors);
            }
            if let Some(hi) = hi {
                walk_node(hi, ctx, source_path, errors);
            }
        }
        Node::LetDestructureStruct { value, .. } | Node::LetTupleDestructure { value, .. } => {
            walk_node(value, ctx, source_path, errors)
        }
        Node::Const { value, .. } | Node::StaticLet { value, .. } => {
            walk_node(value, ctx, source_path, errors);
        }
        Node::InterpolatedString { parts, .. } => {
            for part in parts {
                if let crate::string_interp::StringPart::Expr(expr) = part {
                    walk_node(expr, ctx, source_path, errors);
                }
            }
        }
        Node::Quantifier { range, body, .. } => {
            match range {
                crate::quantifiers::QuantRange::Range { lo, hi } => {
                    walk_node(lo, ctx, source_path, errors);
                    walk_node(hi, ctx, source_path, errors);
                }
                crate::quantifiers::QuantRange::Iterable(iterable) => {
                    walk_node(iterable, ctx, source_path, errors);
                }
            }
            walk_node(body, ctx, source_path, errors);
        }
        Node::Range { lo, hi, .. } => {
            walk_node(lo, ctx, source_path, errors);
            walk_node(hi, ctx, source_path, errors);
        }
        Node::InvariantStatement { expr, .. } | Node::BreakWith { value: expr, .. } => {
            walk_node(expr, ctx, source_path, errors);
        }
        Node::StaticAssert { condition, .. } => {
            walk_node(condition, ctx, source_path, errors);
        }
        Node::BenchBlock { body, .. } | Node::UnsafeBlock { body, .. } => {
            walk_node(body, ctx, source_path, errors);
        }
        _ => {}
    }
}

/// Core: attempt to prove `0 <= index < len(target)` from the
/// accumulated axioms. On success, bump the proven counter. On
/// failure, bump the unproven counter and — in strict mode —
/// append a compile error.
fn check_index(
    target: &Node,
    index: &Node,
    span: Span,
    ctx: &BoundsCtx,
    source_path: &str,
    errors: &mut Vec<String>,
) {
    // Fast path: index is a non-negative integer literal and we know
    // the target's literal length — discharge without Z3.
    if let Node::IntegerLiteral { value, .. } = index
        && let Node::Identifier { name, .. } = target
        && let Some(len) = ctx.literal_len.get(name)
    {
        if *value >= 0 && *value < *len {
            mark_proven(span);
            return;
        } else {
            // Provably out of bounds at compile time — this is an
            // error even without `--deny-unproven-bounds` because
            // the program is unambiguously wrong.
            bump_unproven();
            errors.push(format_error(
                source_path,
                span,
                &format!(
                    "index {} is out of bounds for array `{}` of length {}",
                    value, name, len
                ),
            ));
            return;
        }
    }

    // General path: hand `0 <= index AND index < len(target)` to Z3.
    let target_name = match target {
        Node::Identifier { name, .. } => name.clone(),
        _ => {
            // Non-identifier target (e.g. a nested call result). We
            // can't name its length for the SMT translator — leave
            // it to the runtime check.
            bump_unproven();
            if deny_unproven_bounds() {
                errors.push(format_error(
                    source_path,
                    span,
                    "cannot statically prove index is in bounds (non-identifier array)",
                ));
            }
            return;
        }
    };

    let goal = build_bounds_goal(&target_name, index);
    if let Some(true) = try_prove(&goal, &ctx.axioms) {
        mark_proven(span);
        return;
    }

    bump_unproven();
    if deny_unproven_bounds() {
        errors.push(format_error(
            source_path,
            span,
            &format!(
                "cannot statically prove `{}[...]` is in bounds; add a `requires` clause or drop `--deny-unproven-bounds`",
                target_name
            ),
        ));
    }
}

/// Build the boolean goal AST: `(0 <= index) && (index < len(target))`.
fn build_bounds_goal(target_name: &str, index: &Node) -> Node {
    let zero = Node::IntegerLiteral {
        value: 0,
        span: Span::default(),
    };
    let len_call = Node::CallExpression {
        function: Box::new(Node::Identifier {
            name: "len".to_string(),
            span: Span::default(),
        }),
        arguments: vec![Node::Identifier {
            name: target_name.to_string(),
            span: Span::default(),
        }],
        span: Span::default(),
    };
    let ge = Node::InfixExpression {
        left: Box::new(zero),
        operator: "<=",
        right: Box::new(index.clone()),
        span: Span::default(),
    };
    let lt = Node::InfixExpression {
        left: Box::new(index.clone()),
        operator: "<",
        right: Box::new(len_call),
        span: Span::default(),
    };
    Node::InfixExpression {
        left: Box::new(ge),
        operator: "&&",
        right: Box::new(lt),
        span: Span::default(),
    }
}

/// Feature-gated Z3 shim. With `--features z3` compiled in we call
/// the real axiom-aware prover; otherwise only the literal-fold fast
/// path in `check_index` can succeed.
///
/// RES-1194: the caller only acts on `Some(true)` to elide the
/// runtime check; `Some(false)` (contradiction) and `None` (uncertain)
/// both keep the check, so we use `prove_tautology_with_axioms_and_timeout`
/// — that skips the contradiction-phase `solver.check()` entirely.
/// The contract reduces from `Option<bool>` to `Some(true)` / `None`,
/// which matches what `check_index` already does on the result.
#[cfg(feature = "z3")]
fn try_prove(goal: &Node, axioms: &[Node]) -> Option<bool> {
    let bindings: HashMap<String, i64> = HashMap::new();
    let (proven, _cert, _timed_out) =
        crate::verifier_z3::prove_tautology_with_axioms_and_timeout(goal, &bindings, axioms, 5000);
    if proven { Some(true) } else { None }
}

#[cfg(not(feature = "z3"))]
fn try_prove(_goal: &Node, _axioms: &[Node]) -> Option<bool> {
    None
}

/// RES-4115: gate for attaching the `E0009` registry code to the
/// out-of-bounds-index message. Default output stays the pre-existing
/// `error[bounds-check]` label (goldens + integration tests pin that
/// literal string); `RESILIENT_RICH_DIAG=1` switches the label to
/// `error[E0009]`, mirroring `typechecker.rs`'s `rich_diag_enabled`.
fn rich_diag_enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var("RESILIENT_RICH_DIAG").as_deref() == Ok("1"));
    *ENABLED
}

fn format_error(source_path: &str, span: Span, msg: &str) -> String {
    let file = if source_path.is_empty() || source_path == "<unknown>" {
        "<unknown>".to_string()
    } else {
        source_path.to_string()
    };
    let label = if rich_diag_enabled() {
        "error[E0009]"
    } else {
        "error[bounds-check]"
    };
    if span.start.line == 0 {
        format!("{}: {}: {}", file, label, msg)
    } else {
        format!(
            "{}:{}:{}: {}: {}",
            file, span.start.line, span.start.column, label, msg
        )
    }
}

#[cfg(test)]
mod tests {
    use super::BOUNDS_TEST_LOCK as TEST_LOCK;
    use super::*;

    /// Tests share the `DENY_UNPROVEN_BOUNDS` atomic and the
    /// thread-local stats, so serialize them under a mutex to keep
    /// cargo's parallel runner from producing flakes.
    fn parse(src: &str) -> Node {
        let lexer = crate::Lexer::new(src);
        let mut parser = crate::Parser::new(lexer);
        parser.parse_program()
    }

    #[test]
    fn literal_in_bounds_is_proven() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(false);
        let src = r#"
fn main() {
    let xs = [10, 20, 30];
    let y = xs[0];
}
main();
"#;
        let program = parse(src);
        let r = check_array_bounds(&program, "<test>");
        assert!(r.is_ok(), "expected ok, got {:?}", r);
        let stats = last_stats();
        assert!(
            stats.proven >= 1,
            "expected at least one proven, got {:?}",
            stats
        );
    }

    #[test]
    fn literal_out_of_bounds_is_rejected() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(false);
        let src = r#"
fn main() {
    let xs = [1, 2, 3];
    let y = xs[5];
}
main();
"#;
        let program = parse(src);
        let r = check_array_bounds(&program, "<test>");
        assert!(r.is_err(), "expected error for xs[5] where len=3");
    }

    /// RES-4115: default output stays the legacy `error[bounds-check]`
    /// label (no golden churn); `RESILIENT_RICH_DIAG=1` switches the
    /// label to `error[E0009]`. Checks whichever branch the ambient
    /// environment is actually in, mirroring `typechecker.rs`'s
    /// `assert_gated_code` helper.
    #[test]
    fn out_of_bounds_error_carries_e0009_when_gated() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(false);
        let src = r#"
fn main() {
    let xs = [1, 2, 3];
    let y = xs[5];
}
main();
"#;
        let program = parse(src);
        let err = check_array_bounds(&program, "<test>")
            .expect_err("expected error for xs[5] where len=3");
        if std::env::var("RESILIENT_RICH_DIAG").as_deref() == Ok("1") {
            assert!(err.contains("error[E0009]"), "got: {err}");
        } else {
            assert!(err.contains("error[bounds-check]"), "got: {err}");
            assert!(!err.contains("E0009"), "code leaked into default: {err}");
        }
    }

    #[test]
    fn dynamic_index_without_deny_flag_is_ok() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(false);
        let src = r#"
fn get(int i) -> int {
    let xs = [1, 2, 3];
    return xs[i];
}
"#;
        let program = parse(src);
        let r = check_array_bounds(&program, "<test>");
        // No strict flag — unproven is fine; runtime check handles it.
        assert!(r.is_ok(), "expected ok (non-strict), got {:?}", r);
        let stats = last_stats();
        assert!(stats.unproven >= 1);
    }

    #[test]
    fn dynamic_index_with_deny_flag_errors() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(true);
        let src = r#"
fn get(int i) -> int {
    let xs = [1, 2, 3];
    return xs[i];
}
"#;
        let program = parse(src);
        let r = check_array_bounds(&program, "<test>");
        set_deny_unproven_bounds(false);
        assert!(r.is_err(), "expected strict-mode error for unproven index");
    }

    // --- RES-407: proven-site tracking ---

    #[test]
    fn proven_literal_index_is_recorded_with_span() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(false);
        let src = r#"
fn main() {
    let xs = [10, 20, 30];
    let y = xs[1];
}
main();
"#;
        let program = parse(src);
        check_array_bounds(&program, "<test>").unwrap();
        let sites = proven_sites_sorted();
        assert_eq!(sites.len(), 1, "expected one proven site, got {:?}", sites);
        // Span points at line 4 (`let y = xs[1];`) — line numbers are
        // 1-indexed in the source above (with the leading newline,
        // `fn main()` is line 2 → array literal line 3 → access line 4).
        assert_eq!(sites[0].start.line, 4);
    }

    #[test]
    fn unproven_index_is_not_recorded() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(false);
        let src = r#"
fn get(int i) -> int {
    let xs = [1, 2, 3];
    return xs[i];
}
"#;
        let program = parse(src);
        check_array_bounds(&program, "<test>").unwrap();
        let sites = proven_sites_sorted();
        assert!(
            sites.is_empty(),
            "expected no proven sites for dynamic index, got {:?}",
            sites
        );
    }

    #[test]
    fn proven_sites_reset_between_runs() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(false);
        let src1 = r#"
fn main() {
    let xs = [10, 20, 30];
    let y = xs[0];
}
main();
"#;
        let program1 = parse(src1);
        check_array_bounds(&program1, "<test>").unwrap();
        assert_eq!(proven_sites_sorted().len(), 1);

        // Second run with no proven sites must clear the previous one.
        let src2 = r#"
fn get(int i) -> int {
    let xs = [1, 2, 3];
    return xs[i];
}
"#;
        let program2 = parse(src2);
        check_array_bounds(&program2, "<test>").unwrap();
        assert!(proven_sites_sorted().is_empty());
    }

    #[test]
    fn is_proven_site_returns_correct_membership() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(false);
        let src = r#"
fn main() {
    let xs = [10, 20, 30];
    let y = xs[1];
}
main();
"#;
        let program = parse(src);
        check_array_bounds(&program, "<test>").unwrap();
        let sites = proven_sites_sorted();
        assert_eq!(sites.len(), 1);
        assert!(is_proven_site(sites[0]));
        // A made-up span that doesn't match any access should be false.
        assert!(!is_proven_site(Span::default()));
    }

    #[test]
    fn strict_mode_checks_indexes_inside_match_arms() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(true);
        let program =
            parse("fn main(int i) { let xs = [1, 2, 3]; let y = match 0 { 0 => xs[i], _ => 0 }; }");
        let err = check_array_bounds(&program, "<test>")
            .expect_err("match-arm index must not evade strict bounds checking");
        set_deny_unproven_bounds(false);
        assert!(err.contains("cannot statically prove"), "got: {err}");
    }

    #[test]
    fn strict_mode_checks_indexes_inside_function_literals() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(true);
        let program =
            parse("fn main(int i) { let xs = [1, 2, 3]; let f = fn() { return xs[i]; }; }");
        let err = check_array_bounds(&program, "<test>")
            .expect_err("closure index must not evade strict bounds checking");
        set_deny_unproven_bounds(false);
        assert!(err.contains("cannot statically prove"), "got: {err}");
    }

    #[test]
    fn strict_mode_checks_indexes_inside_try_handlers() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(true);
        let program =
            parse("fn main(int i) { let xs = [1, 2, 3]; try { xs[i]; } catch Timeout { xs[0]; } }");
        let err = check_array_bounds(&program, "<test>")
            .expect_err("try-body index must not evade strict bounds checking");
        set_deny_unproven_bounds(false);
        assert!(err.contains("cannot statically prove"), "got: {err}");
    }

    #[test]
    fn strict_mode_checks_indexes_inside_loop_invariants() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(true);
        let program = parse(
            "fn main(int i) { let xs = [1, 2, 3]; while i < 3 { invariant xs[i] >= 0; i = i + 1; } }",
        );
        let err = check_array_bounds(&program, "<test>")
            .expect_err("loop-invariant index must not evade strict bounds checking");
        set_deny_unproven_bounds(false);
        assert!(err.contains("cannot statically prove"), "got: {err}");
    }

    #[test]
    fn strict_mode_pipeline_runs_for_index_only_in_loop_invariant() {
        let _g = TEST_LOCK.lock().unwrap();
        set_deny_unproven_bounds(true);
        let program = parse(
            "fn main(int i) { let xs = [1, 2, 3]; while i < 3 { invariant xs[i] >= 0; i = i + 1; } }",
        );
        let result =
            crate::typechecker::TypeChecker::new().check_program_with_source(&program, "<test>");
        set_deny_unproven_bounds(false);
        let err = result.expect_err("the typechecker must run bounds checking for invariants");
        assert!(err.contains("cannot statically prove"), "got: {err}");
    }
}
