//! RES-318: Z3 inductive verification of `invariant` annotations on
//! `while` loops. Standard Hoare-logic loop rule:
//!
//! 1. **Base**: prove the invariant holds before the first iteration.
//! 2. **Inductive**: assume the invariant + the loop condition; prove
//!    the invariant still holds after the body executes.
//!
//! When both goals discharge, certificate emission writes one isolated
//! SMT-LIB2 query per base or inductive obligation. The stats counter
//! still counts proven invariants. With `--verbose` set,
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
    // RES-1297 / RES-1917: the typechecker gates this call behind
    // statement-form, while, or finite-loop invariant markers,
    // so the program is guaranteed to contain at least one invariant.
    // The previous `any_node` pre-scan was redundant — removed.
    let timeout_ms = tc.verifier_timeout_ms();
    let verbose = tc.verbose_loop_invariants();
    // RES-1798: pre-size to 8 — bindings accumulate Let/Const literal-
    // int bindings as `walk` descends. Programs that reach this point
    // (have at least one invariant) typically declare 2-10 const
    // bindings; 8 covers the common case without rehash churn.
    let mut bindings: HashMap<String, i64> = HashMap::with_capacity(8);
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
            invalidate_mutations(node, bindings);
        }
        Node::Function { body, .. } | Node::FunctionLiteral { body, .. } => {
            // Each fn body opens a fresh scope. Pre-loop constants
            // declared inside the fn become bindings just for that
            // fn; the outer caller's bindings are NOT visible (we
            // don't track which globals get shadowed by params).
            // RES-1798: pre-size to 8 — same shape as the outer
            // bindings map. Each fn body opens a fresh scope.
            let mut fn_bindings: HashMap<String, i64> = HashMap::with_capacity(8);
            walk(body, &mut fn_bindings, tc, next_idx, timeout_ms, verbose);
        }
        Node::LetStatement { name, value, .. } | Node::Const { name, value, .. } => {
            invalidate_mutations(value, bindings);
            if let Some(v) = literal_int(value) {
                bindings.insert(name.clone(), v);
            } else {
                // Non-literal RHS — drop any prior binding so we
                // don't carry a stale value.
                bindings.remove(name);
            }
        }
        Node::Assignment { name, value, .. } => {
            invalidate_mutations(value, bindings);
            bindings.remove(name);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            invalidate_mutations(condition, bindings);
            let snap = bindings.clone();
            walk(consequence, bindings, tc, next_idx, timeout_ms, verbose);
            match alternative {
                Some(alt) => {
                    *bindings = snap.clone();
                    walk(alt, bindings, tc, next_idx, timeout_ms, verbose);
                    *bindings = snap;
                }
                None => {
                    *bindings = snap;
                }
            }
            invalidate_mutations(node, bindings);
        }
        Node::WhileStatement {
            condition,
            body,
            invariants: pre_invariants,
            span,
            ..
        } => {
            invalidate_mutations(condition, bindings);
            let entry_bindings = bindings.clone();
            // A nested loop can run after any number of outer iterations.
            invalidate_mutations(body, bindings);
            walk(body, bindings, tc, next_idx, timeout_ms, verbose);
            invalidate_mutations(body, bindings);

            // Collect the full invariant list — both pre-body
            // (RES-132a) and statement-form (RES-222) variants.
            //
            // RES-1394: chain the two sources directly into the
            // try_prove_invariant loop instead of materialising an
            // intermediate `Vec<&Node>`. Saves one Vec allocation +
            // 2N pushes per WhileStatement with invariants. The
            // emptiness short-circuit now checks the two sources
            // directly (same semantic as the old `all.is_empty()`).
            let body_invs = crate::loop_invariants::collect_body_invariants(body);
            if pre_invariants.is_empty() && body_invs.is_empty() {
                return;
            }

            // Compute the set of names assigned in the body — these
            // become free in the inductive step.
            let mut assigned: BTreeSet<&str> = BTreeSet::new();
            collect_assigned_names(body, &mut assigned);

            for inv in pre_invariants.iter().chain(body_invs.iter().copied()) {
                try_prove_invariant(
                    tc,
                    inv,
                    condition,
                    body,
                    &entry_bindings,
                    &assigned,
                    span,
                    next_idx,
                    timeout_ms,
                    verbose,
                );
            }
        }
        Node::ForInStatement {
            name,
            iterable,
            body,
            invariants: pre_invariants,
            span,
            ..
        } => {
            invalidate_mutations(iterable, bindings);
            let entry_bindings = bindings.clone();
            invalidate_mutations(body, bindings);
            bindings.remove(name);
            walk(body, bindings, tc, next_idx, timeout_ms, verbose);
            invalidate_mutations(body, bindings);

            let body_invs = crate::loop_invariants::collect_body_invariants(body);
            if pre_invariants.is_empty() && body_invs.is_empty() {
                return;
            }

            // Keep the model finite and auditable: only literal integer
            // arrays are expanded. All other iterables retain the runtime
            // check without receiving a static proof.
            let Some(elements) = literal_int_array(iterable) else {
                return;
            };
            for inv in pre_invariants.iter().chain(body_invs.iter().copied()) {
                try_prove_for_in_invariant(
                    tc,
                    inv,
                    name,
                    body,
                    &elements,
                    &entry_bindings,
                    span,
                    next_idx,
                    timeout_ms,
                    verbose,
                );
            }
        }
        Node::LiveBlock { body, .. } => {
            walk(body, bindings, tc, next_idx, timeout_ms, verbose);
        }
        Node::TryCatch { body, handlers, .. } => {
            // A handler can start after any prefix of the protected body.
            invalidate_mutations(node, bindings);
            let entry_bindings = bindings.clone();
            for s in body {
                walk(s, bindings, tc, next_idx, timeout_ms, verbose);
            }
            for (_v, h) in handlers {
                *bindings = entry_bindings.clone();
                for s in h {
                    walk(s, bindings, tc, next_idx, timeout_ms, verbose);
                }
            }
            *bindings = entry_bindings;
            invalidate_mutations(node, bindings);
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                walk(m, bindings, tc, next_idx, timeout_ms, verbose);
            }
        }
        _ => invalidate_mutations(node, bindings),
    }
}

#[cfg(feature = "z3")]
fn invalidate_mutations(node: &Node, bindings: &mut HashMap<String, i64>) {
    crate::uniqueness_walk::visit(node, &mut |child| match child {
        Node::Assignment { name, .. } => {
            bindings.remove(name);
        }
        // Calls may mutate captured or global state beyond this local model.
        Node::CallExpression { .. } => bindings.clear(),
        _ => {}
    });
}

/// Run the base + inductive proof for a single invariant. On success,
/// captures separate base and step queries and emits a verbose stderr line.
#[cfg(feature = "z3")]
#[allow(clippy::too_many_arguments)]
fn try_prove_invariant(
    tc: &mut crate::typechecker::TypeChecker,
    invariant: &Node,
    condition: &Node,
    body: &Node,
    bindings: &HashMap<String, i64>,
    assigned: &BTreeSet<&str>,
    loop_span: &Span,
    next_idx: &mut usize,
    timeout_ms: u32,
    verbose: bool,
) {
    let prove = if tc.captures_proofs() {
        crate::verifier_z3::prove_tautology_with_certificate
    } else {
        crate::verifier_z3::prove_tautology_with_axioms_and_timeout
    };
    // -------- Base case --------
    // Bindings include every pre-loop constant. If the invariant
    // is provable from these alone, the entry obligation is met.
    //
    // RES-1194: only the `Some(true)` verdict matters here — the
    // immediate `return` discards everything else identically. Use
    // the tautology-only fast path so Z3 doesn't run the
    // contradiction-phase `solver.check()` whose result we'd
    // throw away. Certificate is still produced for the proven case
    // so the SMT-LIB2 emission below keeps working.
    let (base_proven, base_cert, _timed) = prove(invariant, bindings, &[], timeout_ms);
    if !base_proven || (tc.captures_proofs() && base_cert.is_none()) {
        return;
    }

    // -------- Inductive step --------
    // Drop bindings for names assigned in the body so they're free.
    // `bindings.len()` is the exact upper bound — `filter` only ever
    // shrinks the count — and `HashMap::FromIterator` over a filtered
    // iterator pulls only the lower bound (0) from `size_hint`, so the
    // collected map starts at capacity 0 and grows. Building it with
    // an explicit pre-sized loop skips that grow cascade.
    let mut step_bindings: HashMap<String, i64> = HashMap::with_capacity(bindings.len());
    for (k, v) in bindings.iter() {
        if !assigned.contains(k.as_str()) {
            step_bindings.insert(k.clone(), *v);
        }
    }

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
    //
    // RES-1194: same tautology-only path as the base case — the
    // `return` below discards every non-`Some(true)` verdict
    // identically.
    let (step_proven, step_cert, _timed) = prove(&goal, &step_bindings, &[], timeout_ms);
    if !step_proven || (tc.captures_proofs() && step_cert.is_none()) {
        return;
    }

    let idx = *next_idx;
    *next_idx += 1;
    tc.push_loop_invariant_certificates(
        idx,
        base_cert.map_or_else(String::new, |cert| cert.smt2),
        step_cert.into_iter().map(|cert| cert.smt2).collect(),
    );

    let inv_span = invariant_span(invariant).unwrap_or(loop_span);
    if verbose {
        eprintln!(
            "-- invariant proven, runtime check elided at {}:{}",
            inv_span.start.line, inv_span.start.column
        );
    }
}

/// Symbolically execute a finite literal array from the known entry state.
/// Invariants are checked after each element is bound, before the body runs.
/// The base query checks the first entry; each step checks the current entry
/// and (when present) the next entry after body execution and rebinding. These
/// overlapping checkpoints preserve the base/step capture layout; they are
/// not distinct source invariants or a requirement on the final body state.
#[cfg(feature = "z3")]
#[allow(clippy::too_many_arguments)]
fn try_prove_for_in_invariant(
    tc: &mut crate::typechecker::TypeChecker,
    invariant: &Node,
    loop_name: &str,
    body: &Node,
    elements: &[i64],
    bindings: &HashMap<String, i64>,
    loop_span: &Span,
    next_idx: &mut usize,
    timeout_ms: u32,
    verbose: bool,
) {
    // There is no executed checkpoint for an empty iterable, so no proof is
    // emitted. Bound trace expansion keeps best-effort verification finite.
    if elements.is_empty() || elements.len() > 128 {
        return;
    }
    let Node::Block { stmts, .. } = body else {
        return;
    };
    let mut state: HashMap<String, Node> = bindings
        .iter()
        .map(|(name, value)| (name.clone(), integer_node(*value)))
        .collect();
    state.insert(loop_name.to_string(), integer_node(elements[0]));
    let Some(first_entry) = finite_state_expression(invariant, &state) else {
        return;
    };
    let prove = if tc.captures_proofs() {
        crate::verifier_z3::prove_tautology_with_certificate
    } else {
        crate::verifier_z3::prove_tautology_with_axioms_and_timeout
    };
    // State expressions already capture entry constants. Passing the outer
    // bindings again would reintroduce a shadowed iteration name.
    let no_bindings = HashMap::new();
    let (base_proven, base_cert, _) = prove(&first_entry, &no_bindings, &[], timeout_ms);
    if !base_proven || (tc.captures_proofs() && base_cert.is_none()) {
        return;
    }
    let mut current_entry = first_entry;
    let mut step_certs = Vec::with_capacity(elements.len());
    for index in 0..elements.len() {
        for stmt in stmts {
            match stmt {
                Node::Assignment { name, value, .. } => {
                    let Some(value) = finite_state_expression(value, &state) else {
                        return;
                    };
                    state.insert(name.clone(), value);
                }
                Node::InvariantStatement { .. } => {}
                _ => return,
            }
        }
        let next_entry = if let Some(element) = elements.get(index + 1) {
            // A write to the iteration variable belongs to this iteration;
            // the next actual element replaces it before the next check.
            state.insert(loop_name.to_string(), integer_node(*element));
            let Some(entry) = finite_state_expression(invariant, &state) else {
                return;
            };
            Some(entry)
        } else {
            None
        };
        let goal = match &next_entry {
            Some(entry) => Node::InfixExpression {
                left: Box::new(current_entry.clone()),
                operator: "&&",
                right: Box::new(entry.clone()),
                span: Span::default(),
            },
            None => current_entry.clone(),
        };
        let (proven, cert, _) = prove(&goal, &no_bindings, &[], timeout_ms);
        if !proven || (tc.captures_proofs() && cert.is_none()) {
            return;
        }
        if let Some(cert) = cert {
            step_certs.push(cert.smt2);
        }
        if let Some(entry) = next_entry {
            current_entry = entry;
        }
    }
    let idx = *next_idx;
    *next_idx += 1;
    tc.push_loop_invariant_certificates(
        idx,
        base_cert.map_or_else(String::new, |cert| cert.smt2),
        step_certs,
    );
    let inv_span = invariant_span(invariant).unwrap_or(loop_span);
    if verbose {
        eprintln!(
            "-- invariant proven, runtime check elided at {}:{}",
            inv_span.start.line, inv_span.start.column
        );
    }
}

#[cfg(feature = "z3")]
fn integer_node(value: i64) -> Node {
    Node::IntegerLiteral {
        value,
        span: Span::default(),
    }
}

/// Substitute a snapshot simultaneously: stored expressions refer to the
/// original entry state and must never be rebound by a later assignment.
#[cfg(feature = "z3")]
fn finite_state_expression(node: &Node, state: &HashMap<String, Node>) -> Option<Node> {
    let expression = match node {
        Node::Identifier { name, .. } => state.get(name).unwrap_or(node).clone(),
        Node::IntegerLiteral { .. } | Node::BooleanLiteral { .. } => node.clone(),
        Node::InfixExpression {
            left,
            operator,
            right,
            span,
        } => Node::InfixExpression {
            left: Box::new(finite_state_expression(left, state)?),
            operator,
            right: Box::new(finite_state_expression(right, state)?),
            span: *span,
        },
        Node::PrefixExpression {
            operator,
            right,
            span,
        } => Node::PrefixExpression {
            operator,
            right: Box::new(finite_state_expression(right, state)?),
            span: *span,
        },
        _ => return None,
    };
    let mut nodes = 0;
    crate::uniqueness_walk::visit(&expression, &mut |_| nodes += 1);
    (nodes <= 32_768).then_some(expression)
}

/// `let NAME = INT_LITERAL` — extract the literal value.
#[cfg(feature = "z3")]
fn literal_int(node: &Node) -> Option<i64> {
    if let Node::IntegerLiteral { value, .. } = node {
        Some(*value)
    } else if let Node::PrefixExpression {
        operator, right, ..
    } = node
        && *operator == "-"
        && let Node::IntegerLiteral { value, .. } = right.as_ref()
    {
        Some(-*value)
    } else {
        None
    }
}

/// Extract a finite integer array for the conservative `for-in` model.
#[cfg(feature = "z3")]
fn literal_int_array(node: &Node) -> Option<Vec<i64>> {
    let Node::ArrayLiteral { items, .. } = node else {
        return None;
    };
    items.iter().map(literal_int).collect()
}

/// Walk every assignment in `node` and accumulate the LHS names.
/// Conservative — we walk past control flow because any reachable
/// assignment counts. Unsupported assignment forms (`FieldAssignment`,
/// `IndexAssignment`) record nothing; the WP step will bail on those
/// bodies anyway.
///
/// RES-2154: `out` now stores `&'a str` borrowed from the body AST
/// instead of owning a `String` per assignment target. The only
/// consumer (`try_prove_invariant`) reads the set via
/// `assigned.contains(k.as_str())`, which works unchanged because
/// `&str: Borrow<str>`. Eliminates one `String::clone()` per
/// `Node::Assignment` reached during the per-loop scan.
#[cfg(feature = "z3")]
fn collect_assigned_names<'a>(node: &'a Node, out: &mut BTreeSet<&'a str>) {
    match node {
        Node::Assignment { name, .. } => {
            out.insert(name.as_str());
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
///
/// RES-2280: fast-reject assignments whose LHS is not referenced in
/// the current postcondition. `substitute` is otherwise forced to
/// walk the whole AST and clone every node it visits — for a body
/// of S assignments and a postcondition of P nodes, that's O(S*P)
/// deep clones even when most assignments target variables the
/// postcondition never mentions. Probing with `contains_ident`
/// (a non-allocating walker) lets the unreferenced-name case skip
/// the clone storm entirely; the referenced-name path still pays
/// the full substitute cost.
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
                if !contains_ident(&current, name) {
                    continue;
                }
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

/// Non-allocating walker — returns `true` as soon as a free
/// occurrence of `name` is found in `node`. Mirrors the helper of
/// the same name in `secret_erasure`, scoped to the WP module so
/// the LIA / BV32 translator subset stays self-contained.
#[cfg(feature = "z3")]
fn contains_ident(node: &Node, name: &str) -> bool {
    crate::uniqueness_walk::any_node(
        node,
        |n| matches!(n, Node::Identifier { name: n_name, .. } if n_name == name),
    )
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
            operator,
            right: Box::new(substitute(right, name, replacement)),
            span: *span,
        },
        Node::PrefixExpression {
            operator,
            right,
            span,
        } => Node::PrefixExpression {
            operator,
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
        operator: "&&",
        right: Box::new(cond.clone()),
        span: Span::default(),
    };
    let neg_p_and_cond = Node::PrefixExpression {
        operator: "!",
        right: Box::new(p_and_cond),
        span: Span::default(),
    };
    Node::InfixExpression {
        left: Box::new(neg_p_and_cond),
        operator: "||",
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
        let lexer = Lexer::new(src);
        let mut parser = Parser::new(lexer);
        parser.parse_program()
    }

    fn run(src: &str) -> usize {
        let p = parse(src);
        // Run the body of the loop_invariants check first so a bad
        // program doesn't reach the verifier with an out-of-loop
        // `invariant` statement.
        crate::loop_invariants::check(&p, "<test>").expect("loop_invariants check failed");
        // RES-1357: opt into cert capture so
        // `loop_invariant_certificate_count` reflects the proven
        // count (the production path leaves it off so the Vec push
        // doesn't fire on hot paths that don't read it).
        let mut tc = TypeChecker::new()
            .with_warn_unverified(false)
            .with_proof_capture(true);
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
    fn for_in_literal_integer_invariant_is_proven() {
        // A finite integer array can be checked one element at a time.
        let src = r#"
            let s = 0;
            for x in [1, 2, 3] invariant s >= 0 {
                s = s + x;
            }
        "#;
        let proven = run(src);
        assert_eq!(proven, 1);
    }

    #[test]
    fn for_in_literal_integer_invariant_counterexample_is_not_proven() {
        let src = r#"
            let s = 0;
            for x in [1, 2, 3] invariant s <= 2 {
                s = s + x;
            }
        "#;
        let proven = run(src);
        assert_eq!(proven, 0);
    }

    #[test]
    fn for_in_non_literal_iterable_stays_unproven() {
        let src = r#"
            let values = [1, 2, 3];
            let s = 0;
            for x in values invariant s >= 0 {
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

#[cfg(all(test, feature = "z3"))]
mod binding_soundness_tests;
