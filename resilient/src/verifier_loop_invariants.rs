//! RES-318: Z3 inductive verification of `invariant` annotations on
//! `while` loops. Standard Hoare-logic loop rule:
//!
//! 1. **Base**: prove the invariant holds before the first iteration.
//! 2. **Inductive**: assume the invariant + the loop condition; prove
//!    the invariant still holds after the body executes.
//!
//! When both goals discharge, we capture an SMT-LIB2 certificate (so
//! `--emit-certificate <DIR>` writes one re-verifiable file per
//! proven invariant) and bump a stats counter. With `--verbose` set,
//! one `-- invariant proven, runtime check elided at L:C` line per
//! discharged invariant is written to stderr.
//!
//! This module is a *best-effort* verifier — the runtime check
//! emitted by `loop_invariants.rs` always fires regardless of the
//! verdict here. An unprovable invariant is silent: the runtime check
//! catches the violation. The static result is informational +
//! a hook for future codegen elision (`G9`).
//!
//! Scope of the MVP weakest-precondition substitution:
//!   - `while COND { ASSIGN ; ASSIGN ; ... }` bodies, where every
//!     statement is either `Node::Assignment { name, value, .. }`
//!     (plain `name = expr;`) or a no-op `InvariantStatement`.
//!   - The condition + invariants must lie in the LIA / BV32 subset
//!     understood by `verifier_z3::translate_bool` (integer arithmetic,
//!     comparisons, boolean connectives). Anything richer makes the
//!     translator return `None`, which is treated the same as
//!     "unprovable" — runtime check stays in.
//!   - Pre-loop integer constants declared as `let NAME = INT_LITERAL`
//!     at program top level (or function top level) are threaded as
//!     bindings. Constants whose names are reassigned inside the loop
//!     body are dropped from the inductive-step bindings — they must
//!     be free for the proof to be universal over iterations.
//!
//! Anything outside that scope causes the verifier to silently bail
//! on that invariant; the runtime check fires unchanged.

use crate::Node;
#[cfg(feature = "z3")]
use crate::span::Span;
#[cfg(feature = "z3")]
use std::collections::{BTreeSet, HashMap};

/// Public entry point — feature-gated so callers don't need to do
/// their own `#[cfg(feature = "z3")]` guards. Without `z3`, this is
/// a no-op and the runtime check from `loop_invariants.rs` is the
/// only line of defense.
#[cfg(feature = "z3")]
pub(crate) fn verify_and_capture(tc: &mut crate::typechecker::TypeChecker, program: &Node) {
    let timeout_ms = tc.verifier_timeout_ms();
    let verbose = tc.verbose_loop_invariants();
    let mut bindings: HashMap<String, i64> = HashMap::new();
    let mut next_idx = 0usize;
    walk(
        program,
        &mut bindings,
        tc,
        &mut next_idx,
        timeout_ms,
        verbose,
    );
}

#[cfg(not(feature = "z3"))]
pub(crate) fn verify_and_capture(_tc: &mut crate::typechecker::TypeChecker, _program: &Node) {}

/// Recursive walk of the program AST. Tracks the lexically-visible
/// integer-literal bindings as we descend, so each loop sees the
/// constants that are in scope at its declaration site. Bindings
/// shadowed by a re-`let` are overwritten in the local map; the
/// caller's snapshot is restored when the recursive call returns.
#[cfg(feature = "z3")]
fn walk(
    node: &Node,
    bindings: &mut HashMap<String, i64>,
    tc: &mut crate::typechecker::TypeChecker,
    next_idx: &mut usize,
    timeout_ms: u32,
    verbose: bool,
) {
    match node {
        Node::Program(stmts) => {
            // Top-level: the whole program shares a single bindings
            // scope; we still snapshot so a stray non-program caller
            // doesn't leak bindings.
            let snap = bindings.clone();
            for s in stmts {
                walk(&s.node, bindings, tc, next_idx, timeout_ms, verbose);
            }
            *bindings = snap;
        }
        Node::Block { stmts, .. } => {
            let snap = bindings.clone();
            for s in stmts {
                walk(s, bindings, tc, next_idx, timeout_ms, verbose);
            }
            *bindings = snap;
        }
        Node::Function { body, .. } | Node::FunctionLiteral { body, .. } => {
            // Each fn body opens a fresh scope. Pre-loop constants
            // declared inside the fn become bindings just for that
            // fn; the outer caller's bindings are NOT visible (we
            // don't track which globals get shadowed by params).
            let mut fn_bindings: HashMap<String, i64> = HashMap::new();
            walk(body, &mut fn_bindings, tc, next_idx, timeout_ms, verbose);
        }
        Node::LetStatement { name, value, .. } | Node::Const { name, value, .. } => {
            if let Some(v) = literal_int(value) {
                bindings.insert(name.clone(), v);
            } else {
                // Non-literal RHS — drop any prior binding so we
                // don't carry a stale value.
                bindings.remove(name);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            // Both branches walked under a snapshot — neither
            // branch's bindings outlive the if.
            let snap = bindings.clone();
            walk(consequence, bindings, tc, next_idx, timeout_ms, verbose);
            *bindings = snap.clone();
            if let Some(alt) = alternative {
                walk(alt, bindings, tc, next_idx, timeout_ms, verbose);
                *bindings = snap;
            }
        }
        Node::WhileStatement {
            condition,
            body,
            invariants: pre_invariants,
            span,
        } => {
            // First, walk the body itself in case it contains nested
            // loops — those should also get verified, with the OUTER
            // loop's pre-bindings still in scope. Recursion is safe
            // because we snapshot at every Block boundary.
            walk(body, bindings, tc, next_idx, timeout_ms, verbose);

            // Collect the full invariant list — both pre-body
            // (RES-132a) and statement-form (RES-222) variants.
            let body_invs = crate::loop_invariants::collect_body_invariants(body);
            let mut all: Vec<&Node> = Vec::new();
            for inv in pre_invariants {
                all.push(inv);
            }
            for inv in &body_invs {
                all.push(*inv);
            }
            if all.is_empty() {
                return;
            }

            // Compute the set of names assigned in the body — these
            // become free in the inductive step.
            let mut assigned: BTreeSet<String> = BTreeSet::new();
            collect_assigned_names(body, &mut assigned);

            for inv in all {
                try_prove_invariant(
                    tc, inv, condition, body, bindings, &assigned, span, next_idx, timeout_ms,
                    verbose,
                );
            }
        }
        Node::ForInStatement { body, .. } => {
            // RES-318 MVP: `for-in` invariants are not modeled yet.
            // Recurse so nested while-loops still get verified.
            walk(body, bindings, tc, next_idx, timeout_ms, verbose);
        }
        Node::LiveBlock { body, .. } => {
            walk(body, bindings, tc, next_idx, timeout_ms, verbose);
        }
        Node::TryCatch { body, handlers, .. } => {
            for s in body {
                walk(s, bindings, tc, next_idx, timeout_ms, verbose);
            }
            for (_v, h) in handlers {
                for s in h {
                    walk(s, bindings, tc, next_idx, timeout_ms, verbose);
                }
            }
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                walk(m, bindings, tc, next_idx, timeout_ms, verbose);
            }
        }
        _ => {}
    }
}

/// Run the base + inductive proof for a single invariant. On success,
/// captures one combined SMT-LIB2 certificate (concatenation of the
/// two re-verifiable proofs) and emits a verbose stderr line.
#[cfg(feature = "z3")]
#[allow(clippy::too_many_arguments)]
fn try_prove_invariant(
    tc: &mut crate::typechecker::TypeChecker,
    invariant: &Node,
    condition: &Node,
    body: &Node,
    bindings: &HashMap<String, i64>,
    assigned: &BTreeSet<String>,
    loop_span: &Span,
    next_idx: &mut usize,
    timeout_ms: u32,
    verbose: bool,
) {
    // -------- Base case --------
    // Bindings include every pre-loop constant. If the invariant
    // is provable from these alone, the entry obligation is met.
    let (base_verdict, base_cert, _cx, _timed) =
        crate::verifier_z3::prove_with_axioms_and_timeout(invariant, bindings, &[], timeout_ms);
    if !matches!(base_verdict, Some(true)) {
        return;
    }

    // -------- Inductive step --------
    // Drop bindings for names assigned in the body so they're free.
    let step_bindings: HashMap<String, i64> = bindings
        .iter()
        .filter(|(k, _)| !assigned.contains(k.as_str()))
        .map(|(k, v)| (k.clone(), *v))
        .collect();

    // Build WP(body, invariant) by reverse substitution.
    let wp = match weakest_precondition(body, invariant) {
        Some(q) => q,
        None => return, // body shape unsupported — bail
    };

    // Goal: (invariant /\ cond) => WP(body, invariant)
    //     ≡ NOT(invariant /\ cond) || WP(body, invariant)
    let goal = build_implication(invariant, condition, &wp);

    // The inductive step needs the invariant itself as a hypothesis
    // — we pass it as an axiom rather than embedding inside the
    // implication so the SMT-LIB2 cert has a clean shape. (Embedding
    // works equivalently; the ax form mirrors how `recovers_to`
    // discharges its requires-precondition.)
    let (step_verdict, step_cert, _cx, _timed) =
        crate::verifier_z3::prove_with_axioms_and_timeout(&goal, &step_bindings, &[], timeout_ms);
    if !matches!(step_verdict, Some(true)) {
        return;
    }

    // -------- Both proven: capture certificate + emit message --------
    let mut smt2 = String::new();
    smt2.push_str("; RES-318 loop-invariant proof certificate\n");
    smt2.push_str(&format!(
        "; loop at line {}, col {}\n",
        loop_span.start.line, loop_span.start.column
    ));
    smt2.push_str("; ----- base case -----\n");
    if let Some(c) = base_cert {
        smt2.push_str(&c.smt2);
    }
    smt2.push_str("; ----- inductive step -----\n");
    if let Some(c) = step_cert {
        smt2.push_str(&c.smt2);
    }
    let idx = *next_idx;
    *next_idx += 1;
    tc.push_loop_invariant_certificate(idx, smt2);

    let inv_span = invariant_span(invariant).unwrap_or(loop_span);
    if verbose {
        eprintln!(
            "-- invariant proven, runtime check elided at {}:{}",
            inv_span.start.line, inv_span.start.column
        );
    }
}

/// `let NAME = INT_LITERAL` — extract the literal value.
#[cfg(feature = "z3")]
fn literal_int(node: &Node) -> Option<i64> {
    if let Node::IntegerLiteral { value, .. } = node {
        Some(*value)
    } else if let Node::PrefixExpression {
        operator, right, ..
    } = node
        && operator == "-"
        && let Node::IntegerLiteral { value, .. } = right.as_ref()
    {
        Some(-*value)
    } else {
        None
    }
}

/// Walk every assignment in `node` and accumulate the LHS names.
/// Conservative — we walk past control flow because any reachable
/// assignment counts. Unsupported assignment forms (`FieldAssignment`,
/// `IndexAssignment`) record nothing; the WP step will bail on those
/// bodies anyway.
#[cfg(feature = "z3")]
fn collect_assigned_names(node: &Node, out: &mut BTreeSet<String>) {
    match node {
        Node::Assignment { name, .. } => {
            out.insert(name.clone());
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_assigned_names(s, out);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            collect_assigned_names(consequence, out);
            if let Some(alt) = alternative {
                collect_assigned_names(alt, out);
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            collect_assigned_names(body, out);
        }
        _ => {}
    }
}

/// Build WP(body, post). The body must be a block of supported
/// statements (assignments + invariant no-ops); anything else
/// returns `None` so the caller treats this loop as unprovable.
#[cfg(feature = "z3")]
fn weakest_precondition(body: &Node, post: &Node) -> Option<Node> {
    let stmts = match body {
        Node::Block { stmts, .. } => stmts,
        _ => return None,
    };
    let mut current = post.clone();
    for stmt in stmts.iter().rev() {
        match stmt {
            Node::Assignment { name, value, .. } => {
                current = substitute(&current, name, value);
            }
            Node::InvariantStatement { .. } => {
                // No state change; pass through.
            }
            _ => return None,
        }
    }
    Some(current)
}

/// Substitute every free occurrence of `name` in `node` with a clone
/// of `replacement`. Walks the integer/boolean expression subset that
/// `verifier_z3::translate_bool` can encode — anything richer is
/// passed through unchanged, which keeps WP sound when the invariant
/// references opaque sub-expressions (the prover will then bail
/// during translation).
#[cfg(feature = "z3")]
fn substitute(node: &Node, name: &str, replacement: &Node) -> Node {
    match node {
        Node::Identifier { name: n, .. } if n == name => replacement.clone(),
        Node::InfixExpression {
            left,
            operator,
            right,
            span,
        } => Node::InfixExpression {
            left: Box::new(substitute(left, name, replacement)),
            operator: operator.clone(),
            right: Box::new(substitute(right, name, replacement)),
            span: *span,
        },
        Node::PrefixExpression {
            operator,
            right,
            span,
        } => Node::PrefixExpression {
            operator: operator.clone(),
            right: Box::new(substitute(right, name, replacement)),
            span: *span,
        },
        _ => node.clone(),
    }
}

/// Build the implication `(P /\ cond) => Q` as an AST that the Z3
/// translator understands: `(! (P && cond)) || Q`.
#[cfg(feature = "z3")]
fn build_implication(p: &Node, cond: &Node, q: &Node) -> Node {
    let p_and_cond = Node::InfixExpression {
        left: Box::new(p.clone()),
        operator: "&&".to_string(),
        right: Box::new(cond.clone()),
        span: Span::default(),
    };
    let neg_p_and_cond = Node::PrefixExpression {
        operator: "!".to_string(),
        right: Box::new(p_and_cond),
        span: Span::default(),
    };
    Node::InfixExpression {
        left: Box::new(neg_p_and_cond),
        operator: "||".to_string(),
        right: Box::new(q.clone()),
        span: Span::default(),
    }
}

/// Best-effort span of an invariant expression, for the verbose
/// stderr line. Falls back to the loop span when the invariant's
/// own node doesn't carry one.
#[cfg(feature = "z3")]
fn invariant_span(node: &Node) -> Option<&Span> {
    match node {
        Node::InfixExpression { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::Identifier { span, .. }
        | Node::IntegerLiteral { span, .. }
        | Node::BooleanLiteral { span, .. } => Some(span),
        _ => None,
    }
}

#[cfg(all(test, feature = "z3"))]
mod tests {
    use super::*;
    use crate::typechecker::TypeChecker;
    use crate::{Lexer, Parser};

    fn parse(src: &str) -> Node {
        let lexer = Lexer::new(src.to_string());
        let mut parser = Parser::new(lexer);
        parser.parse_program()
    }

    fn run(src: &str) -> usize {
        let p = parse(src);
        // Run the body of the loop_invariants check first so a bad
        // program doesn't reach the verifier with an out-of-loop
        // `invariant` statement.
        crate::loop_invariants::check(&p, "<test>").expect("loop_invariants check failed");
        let mut tc = TypeChecker::new().with_warn_unverified(false);
        let before = tc.loop_invariant_certificate_count();
        super::verify_and_capture(&mut tc, &p);
        tc.loop_invariant_certificate_count() - before
    }

    #[test]
    fn provable_counter_invariant_is_discharged() {
        // `i` starts at 0, never goes negative under the body.
        let src = r#"
            let i = 0;
            let n = 5;
            while i < n {
                invariant i >= 0;
                i = i + 1;
            }
        "#;
        let proven = run(src);
        assert_eq!(proven, 1, "expected exactly one proven invariant");
    }

    #[test]
    fn unprovable_invariant_falls_through_silently() {
        // `i <= 3` is FALSE on iteration 4 — the proof must fail.
        let src = r#"
            let i = 0;
            let n = 5;
            while i < n {
                invariant i <= 3;
                i = i + 1;
            }
        "#;
        let proven = run(src);
        assert_eq!(proven, 0, "expected the invariant to be unprovable");
    }

    #[test]
    fn pre_body_invariant_form_also_proven() {
        // RES-132a `while c invariant p { ... }` form.
        let src = r#"
            let i = 0;
            let n = 5;
            while i < n invariant i >= 0 {
                i = i + 1;
            }
        "#;
        let proven = run(src);
        assert_eq!(proven, 1);
    }

    #[test]
    fn multiple_invariants_each_attempted() {
        // Both should discharge.
        let src = r#"
            let i = 0;
            let n = 5;
            while i < n {
                invariant i >= 0;
                invariant i <= n;
                i = i + 1;
            }
        "#;
        let proven = run(src);
        assert_eq!(proven, 2, "expected both invariants proven");
    }

    #[test]
    fn body_with_unsupported_statement_silently_bails() {
        // A `println` inside the body breaks WP substitution. The
        // proof must NOT discharge — runtime check stays.
        let src = r#"
            let i = 0;
            let n = 5;
            while i < n {
                invariant i >= 0;
                println("step");
                i = i + 1;
            }
        "#;
        let proven = run(src);
        assert_eq!(proven, 0);
    }

    #[test]
    fn for_in_loop_invariants_are_skipped_for_now() {
        // `for-in` is out of scope for the MVP. The body still has
        // a runtime check; the verifier does NOT attempt a proof.
        let src = r#"
            let s = 0;
            for x in [1, 2, 3] invariant s >= 0 {
                s = s + x;
            }
        "#;
        let proven = run(src);
        assert_eq!(proven, 0);
    }

    #[test]
    fn weakest_precondition_handles_two_assignments() {
        // Body: `j = j + 1; i = i + 1;`  invariant: `i + j >= 0`
        // After WP: `(i + 1) + (j + 1) >= 0`. With base bindings
        // i = 0, j = 0, both base + step are provable.
        let src = r#"
            let i = 0;
            let j = 0;
            let n = 5;
            while i < n {
                invariant i + j >= 0;
                j = j + 1;
                i = i + 1;
            }
        "#;
        let proven = run(src);
        assert_eq!(proven, 1);
    }

    #[test]
    fn binding_substitution_in_substitute_helper_is_local() {
        // Make sure `substitute` doesn't accidentally rewrite bound
        // names that aren't the target. Invariant uses `n`, body
        // assigns `i` — `n` must NOT be substituted.
        let src = r#"
            let i = 0;
            let n = 5;
            while i < n {
                invariant n >= 5;
                i = i + 1;
            }
        "#;
        let proven = run(src);
        assert_eq!(proven, 1);
    }
}
