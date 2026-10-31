//! RES-318: Z3 inductive verification of `invariant` annotations on
//! `while` and finite `for` loops. While-loop induction checks:
//!
//! 1. **Base**: prove the invariant holds before the first iteration.
//! 2. **Inductive**: assume the invariant + the loop condition; prove
//!    the invariant still holds after the body executes.
//!
//! When both goals discharge, certificate emission writes one isolated
//! SMT-LIB2 query per base or inductive obligation. The stats counter
//! still counts proven invariants. With `--verbose` set,
//! one `-- invariant proven, runtime check retained at L:C` line per
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
//!   - Expressions use signed 64-bit bit-vectors with the selected wrap,
//!     saturate, or trap policy. Accepted queries also prove arithmetic
//!     definedness on executed paths. Unsupported expressions make the
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
use std::collections::{BTreeSet, HashMap, HashSet};

#[cfg(feature = "z3")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Integer,
    IntegerFunction,
    Unknown,
}
#[cfg(feature = "z3")]
fn integer_type(ty: &str) -> bool {
    matches!(ty, "int" | "Int" | "i64" | "Int64" | "long")
}
#[cfg(feature = "z3")]
fn expression_sort(node: &Node, sorts: &HashMap<String, Sort>) -> Sort {
    fn visit(node: &Node, sorts: &HashMap<String, Sort>, depth: usize) -> Sort {
        if depth > 128 {
            return Sort::Unknown;
        }
        match node {
            Node::IntegerLiteral { .. } => Sort::Integer,
            Node::Identifier { name, .. } => sorts.get(name).copied().unwrap_or(Sort::Unknown),
            Node::PrefixExpression {
                operator: "-",
                right,
                ..
            } if visit(right, sorts, depth + 1) == Sort::Integer => Sort::Integer,
            Node::InfixExpression {
                left,
                operator,
                right,
                ..
            } if matches!(*operator, "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^")
                && visit(left, sorts, depth + 1) == Sort::Integer
                && visit(right, sorts, depth + 1) == Sort::Integer =>
            {
                Sort::Integer
            }
            Node::CallExpression { function, .. }
                if visit(function, sorts, depth + 1) == Sort::IntegerFunction =>
            {
                Sort::Integer
            }
            _ => Sort::Unknown,
        }
    }
    visit(node, sorts, 0)
}
#[cfg(feature = "z3")]
fn integer_names(sorts: &HashMap<String, Sort>) -> HashSet<String> {
    sorts
        .iter()
        .filter_map(|(name, sort)| (*sort == Sort::Integer).then_some(name.clone()))
        .collect()
}

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
    let mut sorts = HashMap::new();
    if let Node::Program(statements) = program {
        for statement in statements {
            if let Node::Function {
                name,
                return_type: Some(ty),
                ..
            } = &statement.node
                && integer_type(ty)
            {
                sorts.insert(name.clone(), Sort::IntegerFunction);
            }
        }
    }
    let mut next_idx = 0usize;
    walk(
        program,
        &mut bindings,
        &mut sorts,
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
#[allow(clippy::too_many_arguments)]
fn walk(
    node: &Node,
    bindings: &mut HashMap<String, i64>,
    sorts: &mut HashMap<String, Sort>,
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
            let sort_snap = sorts.clone();
            for s in stmts {
                walk(&s.node, bindings, sorts, tc, next_idx, timeout_ms, verbose);
            }
            *bindings = snap;
            sorts.retain(|name, sort| sort_snap.get(name) == Some(sort));
        }
        Node::Block { stmts, .. } => {
            let snap = bindings.clone();
            let sort_snap = sorts.clone();
            for s in stmts {
                walk(s, bindings, sorts, tc, next_idx, timeout_ms, verbose);
            }
            *bindings = snap;
            sorts.retain(|name, sort| sort_snap.get(name) == Some(sort));
            invalidate_mutations(node, bindings, sorts);
        }
        Node::Function {
            body, parameters, ..
        }
        | Node::FunctionLiteral {
            body, parameters, ..
        } => {
            // Each fn body opens a fresh scope. Pre-loop constants
            // declared inside the fn become bindings just for that
            // fn; the outer caller's bindings are NOT visible (we
            // don't track which globals get shadowed by params).
            // RES-1798: pre-size to 8 — same shape as the outer
            // bindings map. Each fn body opens a fresh scope.
            let mut fn_bindings: HashMap<String, i64> = HashMap::with_capacity(8);
            let mut fn_sorts = HashMap::new();
            for (ty, name) in parameters {
                fn_sorts.insert(
                    name.clone(),
                    if integer_type(ty) {
                        Sort::Integer
                    } else {
                        Sort::Unknown
                    },
                );
            }
            walk(
                body,
                &mut fn_bindings,
                &mut fn_sorts,
                tc,
                next_idx,
                timeout_ms,
                verbose,
            );
        }

        Node::LetStatement { name, value, .. } | Node::Const { name, value, .. } => {
            let call_sort = if matches!(value.as_ref(), Node::CallExpression { .. }) {
                expression_sort(value, sorts)
            } else {
                Sort::Unknown
            };
            invalidate_mutations(value, bindings, sorts);
            let sort = if call_sort == Sort::Integer {
                call_sort
            } else {
                expression_sort(value, sorts)
            };
            sorts.insert(name.clone(), sort);
            if let Some(v) = literal_int(value) {
                bindings.insert(name.clone(), v);
            } else {
                // Non-literal RHS — drop any prior binding so we
                // don't carry a stale value.
                bindings.remove(name);
            }
        }
        Node::Assignment { name, value, .. } => {
            invalidate_mutations(value, bindings, sorts);
            sorts.insert(name.clone(), expression_sort(value, sorts));
            bindings.remove(name);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            invalidate_mutations(condition, bindings, sorts);
            let snap = bindings.clone();
            let sort_snap = sorts.clone();
            walk(
                consequence,
                bindings,
                sorts,
                tc,
                next_idx,
                timeout_ms,
                verbose,
            );
            let consequence_sorts = sorts.clone();
            match alternative {
                Some(alt) => {
                    *bindings = snap.clone();
                    *sorts = sort_snap.clone();
                    walk(alt, bindings, sorts, tc, next_idx, timeout_ms, verbose);
                    *bindings = snap;
                    sorts.retain(|name, sort| sort_snap.get(name) == Some(sort));
                }
                None => {
                    *bindings = snap;
                    sorts.retain(|name, sort| sort_snap.get(name) == Some(sort));
                }
            }
            sorts.retain(|name, sort| consequence_sorts.get(name) == Some(sort));
            invalidate_mutations(node, bindings, sorts);
        }
        Node::WhileStatement {
            condition,
            body,
            invariants: pre_invariants,
            span,
            ..
        } => {
            invalidate_mutations(condition, bindings, sorts);
            let entry_bindings = bindings.clone();
            let entry_sorts = sorts.clone();
            // A nested loop can run after any number of outer iterations.
            invalidate_mutations(body, bindings, sorts);
            walk(body, bindings, sorts, tc, next_idx, timeout_ms, verbose);
            invalidate_mutations(body, bindings, sorts);

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
                    &integer_names(&entry_sorts),
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
            invalidate_mutations(iterable, bindings, sorts);
            let entry_bindings = bindings.clone();
            let entry_sorts = sorts.clone();
            invalidate_mutations(body, bindings, sorts);
            bindings.remove(name);
            sorts.insert(
                name.clone(),
                if literal_int_array(iterable).is_some() {
                    Sort::Integer
                } else {
                    Sort::Unknown
                },
            );
            walk(body, bindings, sorts, tc, next_idx, timeout_ms, verbose);
            invalidate_mutations(body, bindings, sorts);
            if let Some(sort) = entry_sorts.get(name) {
                sorts.insert(name.clone(), *sort);
            } else {
                sorts.remove(name);
            }

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
            let mut entry_sorts = entry_sorts;
            entry_sorts.insert(name.clone(), Sort::Integer);
            for inv in pre_invariants.iter().chain(body_invs.iter().copied()) {
                try_prove_for_in_invariant(
                    tc,
                    inv,
                    name,
                    body,
                    &elements,
                    &entry_bindings,
                    &integer_names(&entry_sorts),
                    span,
                    next_idx,
                    timeout_ms,
                    verbose,
                );
            }
        }
        Node::LiveBlock { body, .. } => {
            walk(body, bindings, sorts, tc, next_idx, timeout_ms, verbose);
        }
        Node::TryCatch { body, handlers, .. } => {
            // A handler can start after any prefix of the protected body.
            invalidate_mutations(node, bindings, sorts);
            let entry_bindings = bindings.clone();
            let entry_sorts = sorts.clone();
            for s in body {
                walk(s, bindings, sorts, tc, next_idx, timeout_ms, verbose);
            }
            for (_v, h) in handlers {
                *bindings = entry_bindings.clone();
                *sorts = entry_sorts.clone();
                for s in h {
                    walk(s, bindings, sorts, tc, next_idx, timeout_ms, verbose);
                }
            }
            *bindings = entry_bindings;
            *sorts = entry_sorts;
            invalidate_mutations(node, bindings, sorts);
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                walk(m, bindings, sorts, tc, next_idx, timeout_ms, verbose);
            }
        }
        _ => invalidate_mutations(node, bindings, sorts),
    }
}

#[cfg(feature = "z3")]
fn invalidate_mutations(
    node: &Node,
    bindings: &mut HashMap<String, i64>,
    sorts: &mut HashMap<String, Sort>,
) {
    crate::uniqueness_walk::visit(node, &mut |child| match child {
        Node::Assignment { name, value, .. } => {
            bindings.remove(name);
            // This walk crosses lexical scopes, so it may only discard knowledge.
            if sorts.get(name).copied() != Some(expression_sort(value, sorts)) {
                sorts.remove(name);
            }
        }
        // Calls may mutate captured or global state beyond this local model.
        Node::CallExpression { .. } => {
            bindings.clear();
            sorts.clear();
        }
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
    integers: &HashSet<String>,
    assigned: &BTreeSet<&str>,
    loop_span: &Span,
    next_idx: &mut usize,
    timeout_ms: u32,
    verbose: bool,
) {
    // -------- Base case --------
    // Bindings include every pre-loop constant. If the invariant
    // is provable from these alone, the entry obligation is met.
    //
    let Some(base_cert) = crate::verifier_z3::prove_loop_machine(
        invariant,
        bindings,
        integers,
        None,
        &[],
        timeout_ms,
    ) else {
        return;
    };

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

    let Node::Block { stmts, .. } = body else {
        return;
    };
    let mut expansion_budget = 32_768;
    let mut state: HashMap<String, Node> = step_bindings
        .iter()
        .map(|(name, value)| (name.clone(), integer_node(*value)))
        .collect();
    let mut evaluated = Vec::new();
    for statement in stmts {
        match statement {
            Node::Assignment { name, value, .. } => {
                let Some(value) = finite_state_expression(value, &state, &mut expansion_budget)
                else {
                    return;
                };
                evaluated.push(value.clone());
                state.insert(name.clone(), value);
            }
            Node::InvariantStatement { .. } => {}
            _ => return,
        }
    }
    let steps = if let Some(step_cert) = crate::verifier_z3::prove_loop_machine(
        &wp,
        &step_bindings,
        integers,
        Some((invariant, condition)),
        &evaluated,
        timeout_ms,
    ) {
        vec![step_cert]
    } else {
        let Some(steps) =
            finite_while_trace(invariant, condition, body, bindings, integers, timeout_ms)
        else {
            return;
        };
        steps
    };
    let idx = *next_idx;
    *next_idx += 1;
    tc.push_loop_invariant_certificates(
        idx,
        base_cert,
        if tc.captures_proofs() {
            steps
        } else {
            Vec::new()
        },
    );

    let inv_span = invariant_span(invariant).unwrap_or(loop_span);
    if verbose {
        eprintln!(
            "-- invariant proven, runtime check retained at {}:{}",
            inv_span.start.line, inv_span.start.column
        );
    }
}

/// A concrete terminating trace can establish an invariant that is too weak
/// for induction over arbitrary states. Every entry, branch and executed RHS
/// is checked; reaching the expansion bound never grants proof credit.
#[cfg(feature = "z3")]
fn finite_while_trace(
    invariant: &Node,
    condition: &Node,
    body: &Node,
    bindings: &HashMap<String, i64>,
    integers: &HashSet<String>,
    timeout_ms: u32,
) -> Option<Vec<String>> {
    let Node::Block { stmts, .. } = body else {
        return None;
    };
    let mut expansion_budget = 32_768;
    let mut state: HashMap<String, Node> = bindings
        .iter()
        .map(|(name, value)| (name.clone(), integer_node(*value)))
        .collect();
    let no_bindings = HashMap::new();
    let mut steps = Vec::new();
    let conjunction = |left: Node, right: Node| Node::InfixExpression {
        left: Box::new(left),
        operator: "&&",
        right: Box::new(right),
        span: Span::default(),
    };
    for index in 0..=128 {
        let entry = finite_state_expression(invariant, &state, &mut expansion_budget)?;
        let condition = finite_state_expression(condition, &state, &mut expansion_budget)?;
        let exit = Node::PrefixExpression {
            operator: "!",
            right: Box::new(condition.clone()),
            span: Span::default(),
        };
        if let Some(cert) = crate::verifier_z3::prove_loop_machine(
            &conjunction(entry.clone(), exit),
            &no_bindings,
            integers,
            None,
            &[],
            timeout_ms,
        ) {
            steps.push(format!("; loop-proof-method: finite-while-trace\n{cert}"));
            return Some(steps);
        }
        if index == 128 {
            return None;
        }
        crate::verifier_z3::prove_loop_machine(
            &condition,
            &no_bindings,
            integers,
            None,
            &[],
            timeout_ms,
        )?;
        let mut evaluated = Vec::new();
        for statement in stmts {
            match statement {
                Node::Assignment { name, value, .. } => {
                    let value = finite_state_expression(value, &state, &mut expansion_budget)?;
                    evaluated.push(value.clone());
                    state.insert(name.clone(), value);
                }
                Node::InvariantStatement { .. } => {}
                _ => return None,
            }
        }
        let next = finite_state_expression(invariant, &state, &mut expansion_budget)?;
        let goal = conjunction(conjunction(entry, condition), next);
        let cert = crate::verifier_z3::prove_loop_machine(
            &goal,
            &no_bindings,
            integers,
            None,
            &evaluated,
            timeout_ms,
        )?;
        steps.push(format!("; loop-proof-method: finite-while-trace\n{cert}"));
    }
    None
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
    integers: &HashSet<String>,
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
    let mut expansion_budget = 32_768;
    let mut state: HashMap<String, Node> = bindings
        .iter()
        .map(|(name, value)| (name.clone(), integer_node(*value)))
        .collect();
    state.insert(loop_name.to_string(), integer_node(elements[0]));
    let Some(first_entry) = finite_state_expression(invariant, &state, &mut expansion_budget)
    else {
        return;
    };
    // State expressions already capture entry constants. Passing the outer
    // bindings again would reintroduce a shadowed iteration name.
    let no_bindings = HashMap::new();
    let Some(base_cert) = crate::verifier_z3::prove_loop_machine(
        &first_entry,
        &no_bindings,
        integers,
        None,
        &[],
        timeout_ms,
    ) else {
        return;
    };
    let mut current_entry = first_entry;
    let mut step_certs = Vec::with_capacity(elements.len());
    for index in 0..elements.len() {
        let mut evaluated = Vec::new();
        for stmt in stmts {
            match stmt {
                Node::Assignment { name, value, .. } => {
                    let Some(value) = finite_state_expression(value, &state, &mut expansion_budget)
                    else {
                        return;
                    };
                    evaluated.push(value.clone());
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
            let Some(entry) = finite_state_expression(invariant, &state, &mut expansion_budget)
            else {
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
        let Some(cert) = crate::verifier_z3::prove_loop_machine(
            &goal,
            &no_bindings,
            integers,
            None,
            &evaluated,
            timeout_ms,
        ) else {
            return;
        };
        step_certs.push(cert);
        if let Some(entry) = next_entry {
            current_entry = entry;
        }
    }
    let idx = *next_idx;
    *next_idx += 1;
    tc.push_loop_invariant_certificates(idx, base_cert, step_certs);
    let inv_span = invariant_span(invariant).unwrap_or(loop_span);
    if verbose {
        eprintln!(
            "-- invariant proven, runtime check retained at {}:{}",
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
fn finite_state_expression(
    node: &Node,
    state: &HashMap<String, Node>,
    remaining: &mut usize,
) -> Option<Node> {
    bounded_expression(node, &|name| state.get(name), remaining, 0, true)
}

#[cfg(feature = "z3")]
fn bounded_expression<'a>(
    node: &'a Node,
    lookup: &impl Fn(&str) -> Option<&'a Node>,
    remaining: &mut usize,
    depth: usize,
    replace: bool,
) -> Option<Node> {
    if depth > 128 || *remaining == 0 {
        return None;
    }
    // Charge before allocation, including every node copied from a snapshot.
    *remaining -= 1;
    Some(match node {
        Node::Identifier { name, .. } if replace => match lookup(name) {
            Some(value) => bounded_expression(value, lookup, remaining, depth, false)?,
            None => node.clone(),
        },
        Node::Identifier { .. } | Node::IntegerLiteral { .. } | Node::BooleanLiteral { .. } => {
            node.clone()
        }
        Node::InfixExpression {
            left,
            operator,
            right,
            span,
        } => Node::InfixExpression {
            left: Box::new(bounded_expression(
                left,
                lookup,
                remaining,
                depth + 1,
                replace,
            )?),
            operator,
            right: Box::new(bounded_expression(
                right,
                lookup,
                remaining,
                depth + 1,
                replace,
            )?),
            span: *span,
        },
        Node::PrefixExpression {
            operator,
            right,
            span,
        } => Node::PrefixExpression {
            operator,
            right: Box::new(bounded_expression(
                right,
                lookup,
                remaining,
                depth + 1,
                replace,
            )?),
            span: *span,
        },
        _ => return None,
    })
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
/// Unreferenced assignments need no substitution. Referenced assignments
/// share an allocation budget so repeated duplication cannot expand the
/// postcondition exponentially before the solver sees it.
#[cfg(feature = "z3")]
fn weakest_precondition(body: &Node, post: &Node) -> Option<Node> {
    let stmts = match body {
        Node::Block { stmts, .. } => stmts,
        _ => return None,
    };
    let mut remaining = 32_768;
    let mut current = bounded_expression(post, &|_| None, &mut remaining, 0, false)?;
    for stmt in stmts.iter().rev() {
        match stmt {
            Node::Assignment { name, value, .. } => {
                if !contains_ident(&current, name) {
                    continue;
                }
                current = bounded_expression(
                    &current,
                    &|candidate| (candidate == name).then_some(value.as_ref()),
                    &mut remaining,
                    0,
                    true,
                )?;
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
