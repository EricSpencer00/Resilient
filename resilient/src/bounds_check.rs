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
use std::collections::HashMap;
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

/// Enable `--deny-unproven-bounds` mode. Called from `main.rs` CLI
/// parsing before `check_program_with_source` runs.
pub fn set_deny_unproven_bounds(on: bool) {
    DENY_UNPROVEN_BOUNDS.store(on, Ordering::Relaxed);
}

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
}

/// Read the last-run counters. Tests use this to confirm the pass
/// actually classified the indices it was supposed to.
#[allow(dead_code)]
pub fn last_stats() -> BoundsStats {
    STATS.with(|s| *s.borrow())
}

fn bump_proven() {
    STATS.with(|s| s.borrow_mut().proven += 1);
}
fn bump_unproven() {
    STATS.with(|s| s.borrow_mut().unproven += 1);
}
fn reset_stats() {
    STATS.with(|s| *s.borrow_mut() = BoundsStats::default());
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
    if let Node::Function { body, requires, .. } = node {
        let mut ctx = BoundsCtx::default();
        // Every `requires` clause is an available axiom inside the body.
        for r in requires {
            ctx.axioms.push(r.clone());
        }
        // RES-133b: leading `assume(P)` predicates are also axioms.
        // The runtime check halts before any indexing in the body if
        // they are violated, so the bounds prover may use them.
        ctx.axioms
            .extend(crate::assume_axioms::collect_leading_assume_axioms(body));
        walk_node(body, &ctx, source_path, errors);
    } else if let Node::ImplBlock { methods, .. } = node {
        for m in methods {
            walk_toplevel(m, source_path, errors);
        }
    }
}

fn walk_node(node: &Node, ctx: &BoundsCtx, source_path: &str, errors: &mut Vec<String>) {
    match node {
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
                && operator == ".."
            {
                let ident = |n: &str| Node::Identifier {
                    name: n.to_string(),
                    span: Span::default(),
                };
                // Axiom: lower_bound <= i
                body_ctx.axioms.push(Node::InfixExpression {
                    left: Box::new((**left).clone()),
                    operator: "<=".to_string(),
                    right: Box::new(ident(name)),
                    span: Span::default(),
                });
                // Axiom: i < upper_bound
                body_ctx.axioms.push(Node::InfixExpression {
                    left: Box::new(ident(name)),
                    operator: "<".to_string(),
                    right: Box::new((**right).clone()),
                    span: Span::default(),
                });
            }
            walk_node(iterable, ctx, source_path, errors);
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
            condition, body, ..
        } => {
            walk_node(condition, ctx, source_path, errors);
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
            bump_proven();
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
        bump_proven();
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
        operator: "<=".to_string(),
        right: Box::new(index.clone()),
        span: Span::default(),
    };
    let lt = Node::InfixExpression {
        left: Box::new(index.clone()),
        operator: "<".to_string(),
        right: Box::new(len_call),
        span: Span::default(),
    };
    Node::InfixExpression {
        left: Box::new(ge),
        operator: "&&".to_string(),
        right: Box::new(lt),
        span: Span::default(),
    }
}

/// Feature-gated Z3 shim. With `--features z3` compiled in we call
/// the real axiom-aware prover; otherwise only the literal-fold fast
/// path in `check_index` can succeed.
#[cfg(feature = "z3")]
fn try_prove(goal: &Node, axioms: &[Node]) -> Option<bool> {
    let bindings: HashMap<String, i64> = HashMap::new();
    let (verdict, _cert, _cx, _timed_out) =
        crate::verifier_z3::prove_with_axioms_and_timeout(goal, &bindings, axioms, 5000);
    verdict
}

#[cfg(not(feature = "z3"))]
fn try_prove(_goal: &Node, _axioms: &[Node]) -> Option<bool> {
    None
}

fn format_error(source_path: &str, span: Span, msg: &str) -> String {
    let file = if source_path.is_empty() || source_path == "<unknown>" {
        "<unknown>".to_string()
    } else {
        source_path.to_string()
    };
    if span.start.line == 0 {
        format!("{}: error[bounds-check]: {}", file, msg)
    } else {
        format!(
            "{}:{}:{}: error[bounds-check]: {}",
            file, span.start.line, span.start.column, msg
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Tests share the `DENY_UNPROVEN_BOUNDS` atomic and the
    /// thread-local stats, so serialize them under a mutex to keep
    /// cargo's parallel runner from producing flakes.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn parse(src: &str) -> Node {
        let lexer = crate::Lexer::new(src.to_string());
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
}
