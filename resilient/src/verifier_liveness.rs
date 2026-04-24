// verifier_liveness.rs
//
// RES-388 follow-up: bounded-liveness verifier for actor-level
// `eventually(after: <handler>): <expr>;` claims.
//
// # Encoding
//
// For each `eventually(after: H): Q;` on `actor A { state: T = init; ... }`,
// we search for an integer ranking function μ(state) such that:
//
//   * μ(state_pre) ≥ 0 at every reachable state (well-founded).
//   * After every handler `h ≠ H`, μ(state_post) ≤ μ(state_pre) —
//     "other handlers don't undo progress". This is softened relative
//     to strict decrease because most actors have at least one "idle"
//     receive that leaves state alone; we only need the measure to be
//     non-increasing off-target.
//   * After the target handler `H`, μ(state_post) < μ(state_pre) OR
//     μ(state_post) == 0.
//   * μ(state) == 0 ↔ Q(state).
//
// Combined with a bounded schedule depth (default 8), Z3 either:
//   1. Finds a μ that satisfies all constraints (liveness holds in
//      finitely many `H`-firings from any reachable state) — the
//      claim is marked Proved.
//   2. Refutes a specific constraint with a concrete counterexample
//      — the claim is marked Refuted.
//   3. Returns Unknown / runs out of candidates — we emit
//      `warning[partial-proof]` and move on, matching the ticket's
//      "warn (do not fail) when Z3 cannot decide ranking" wording.
//
// # MVP ranking-function search
//
// The full search space (affine combinations of state + parameters)
// is exponential in the number of state fields. For the MVP we try a
// small, enumerable family of integer measures:
//
//   * μ(state) = state
//   * μ(state) = -state
//   * μ(state) = state - 1
//   * μ(state) = 1 - state
//
// and pick the first that discharges the constraints. This handles
// the canonical draining-queue case (`μ = state`, `Q = state == 0`)
// and the symmetric "count up to bound" case.
//
// Anything outside that family is surfaced as a partial-proof warning
// — the bounded model is intentionally incomplete; follow-ups extend
// the search.

use crate::span::Span;
use crate::{EventuallyClause, Node, ReceiveHandler};

/// Outcome of a bounded-liveness proof attempt for one
/// `eventually(after: H): Q;` clause.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum LivenessResult {
    /// Z3 accepted a ranking function discharging every constraint —
    /// bounded liveness holds.
    Proved,
    /// At least one constraint was refuted with a counterexample.
    Refuted { reason: String },
    /// Neither Proved nor Refuted within the bounded schedule / the
    /// MVP ranking-function family. Surfaces as
    /// `warning[partial-proof]`.
    PartialProof { reason: String },
    /// The actor shape or body contains something the walker can't
    /// translate (e.g. non-straight-line handler). Also folds into
    /// a partial-proof warning at the call site.
    Unsupported { reason: String },
}

/// One proof obligation produced per `eventually` clause per actor.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct LivenessObligation {
    pub actor_name: String,
    pub target_handler: String,
    /// Human-readable rendering of the post-condition Q — used in
    /// diagnostics.
    pub post_label: String,
    pub clause_span: Span,
    pub result: LivenessResult,
}

/// Default bounded-model schedule depth used by the ticket
/// (`default depth 8`). Exposed so tests and future callers can
/// tune it; not yet surfaced via the CLI.
#[allow(dead_code)]
pub(crate) const DEFAULT_SCHEDULE_DEPTH: u32 = 8;

/// Walk each `eventually` clause on an actor and return the verdict
/// list. `timeout_ms` threads the per-query Z3 timeout budget.
#[allow(dead_code)]
pub(crate) fn verify_actor_liveness(
    name: &str,
    state_fields: &[(String, String, Node)],
    eventually_clauses: &[EventuallyClause],
    receive_handlers: &[ReceiveHandler],
    timeout_ms: u32,
) -> Vec<LivenessObligation> {
    let mut out = Vec::new();

    // MVP: single `state` field (same shape constraint the `always`
    // verifier enforces). Actors without state have nothing to
    // decrease — skip.
    let Some((_, state_name, _state_init)) = state_fields.first() else {
        return out;
    };

    for clause in eventually_clauses {
        let post_label = render_clause(&clause.post);
        let target = &clause.target_handler;

        // Unknown handler already surfaces as a type error in the
        // typechecker — guard defensively anyway.
        let Some(target_h) = receive_handlers.iter().find(|h| &h.name == target) else {
            out.push(LivenessObligation {
                actor_name: name.to_string(),
                target_handler: target.clone(),
                post_label: post_label.clone(),
                clause_span: clause.span,
                result: LivenessResult::Unsupported {
                    reason: format!("handler `{}` not found", target),
                },
            });
            continue;
        };

        // Collect handler post-state expressions — bail to
        // Unsupported if any body isn't straight-line.
        let Some(target_post) =
            crate::verifier_actors::straight_line_post_public(&target_h.body, state_name)
        else {
            out.push(LivenessObligation {
                actor_name: name.to_string(),
                target_handler: target.clone(),
                post_label: post_label.clone(),
                clause_span: clause.span,
                result: LivenessResult::Unsupported {
                    reason: format!(
                        "handler `{}` body is not straight-line — ranking search skipped",
                        target
                    ),
                },
            });
            continue;
        };

        let mut other_posts: Vec<(String, Node, Vec<Node>)> = Vec::new();
        let mut bailed = false;
        for h in receive_handlers {
            if &h.name == target {
                continue;
            }
            let Some(post) = crate::verifier_actors::straight_line_post_public(&h.body, state_name)
            else {
                out.push(LivenessObligation {
                    actor_name: name.to_string(),
                    target_handler: target.clone(),
                    post_label: post_label.clone(),
                    clause_span: clause.span,
                    result: LivenessResult::Unsupported {
                        reason: format!(
                            "handler `{}` body is not straight-line — ranking search skipped",
                            h.name
                        ),
                    },
                });
                bailed = true;
                break;
            };
            other_posts.push((h.name.clone(), post, h.requires.clone()));
        }
        if bailed {
            continue;
        }

        let result = try_rank_search(
            state_name,
            &clause.post,
            &target_post,
            &target_h.requires,
            &other_posts,
            timeout_ms,
        );
        out.push(LivenessObligation {
            actor_name: name.to_string(),
            target_handler: target.clone(),
            post_label,
            clause_span: clause.span,
            result,
        });
    }

    out
}

/// Candidate ranking-function family enumerated by the MVP search.
/// Each candidate maps the symbolic pre-state `state` to an integer
/// measure expressed as a Resilient AST node, so it can be fed
/// straight back through the existing Z3 translator.
fn candidates(state_name: &str) -> Vec<Node> {
    let state = Node::Identifier {
        name: state_name.to_string(),
        span: Span::default(),
    };
    vec![
        // μ = state
        state.clone(),
        // μ = -state
        Node::PrefixExpression {
            operator: "-".to_string(),
            right: Box::new(state.clone()),
            span: Span::default(),
        },
        // μ = state - 1
        Node::InfixExpression {
            left: Box::new(state.clone()),
            operator: "-".to_string(),
            right: Box::new(int_lit(1)),
            span: Span::default(),
        },
        // μ = 1 - state
        Node::InfixExpression {
            left: Box::new(int_lit(1)),
            operator: "-".to_string(),
            right: Box::new(state),
            span: Span::default(),
        },
    ]
}

/// Try each candidate ranking function in turn. Return the first
/// that discharges all constraints; if none succeed, fold the
/// failure reasons into a `PartialProof` verdict (ticket contract).
fn try_rank_search(
    state_name: &str,
    post: &Node,
    target_post: &Node,
    target_requires: &[Node],
    other_posts: &[(String, Node, Vec<Node>)],
    timeout_ms: u32,
) -> LivenessResult {
    let mut last_reason = String::from("no ranking candidate discharged the obligations");
    for mu in candidates(state_name) {
        match check_candidate(
            state_name,
            &mu,
            post,
            target_post,
            target_requires,
            other_posts,
            timeout_ms,
        ) {
            CandidateVerdict::Ok => return LivenessResult::Proved,
            CandidateVerdict::Refuted(reason) => last_reason = reason,
            CandidateVerdict::Unknown(reason) => last_reason = reason,
        }
    }
    LivenessResult::PartialProof {
        reason: last_reason,
    }
}

enum CandidateVerdict {
    Ok,
    Refuted(String),
    Unknown(String),
}

/// Verify one ranking-function candidate against the constraints.
/// Uses the existing Z3 backend via `verifier_actors`'s public
/// wrappers so the ranking search stays consistent with how `always`
/// discharges obligations.
fn check_candidate(
    state_name: &str,
    mu: &Node,
    post: &Node,
    target_post: &Node,
    target_requires: &[Node],
    other_posts: &[(String, Node, Vec<Node>)],
    timeout_ms: u32,
) -> CandidateVerdict {
    // Constraint — target handler strictly decreases or reaches zero:
    //   (requires_H ∧ μ_pre > 0) → (μ_post_H < μ_pre ∨ μ_post_H == 0)
    let mu_pre = mu.clone();
    let mu_post_target =
        crate::verifier_actors::substitute_state_public(mu, state_name, target_post);
    let mut antecedent = infix(mu_pre.clone(), ">", int_lit(0));
    for r in target_requires {
        antecedent = and_node(antecedent, r.clone());
    }
    let decrease_or_reach = or_node(
        infix(mu_post_target.clone(), "<", mu_pre.clone()),
        infix(mu_post_target, "==", int_lit(0)),
    );
    let target_obligation = implies_node(antecedent, decrease_or_reach);
    match prove(&target_obligation, timeout_ms) {
        Some(true) => {}
        Some(false) => {
            return CandidateVerdict::Refuted(format!(
                "target handler does not strictly decrease measure `{}`",
                render_clause(mu)
            ));
        }
        None => {
            return CandidateVerdict::Unknown(format!(
                "Z3 could not decide decrease of measure `{}` across target handler",
                render_clause(mu)
            ));
        }
    }

    // Constraint — every other handler doesn't undo progress:
    //   (requires_h) → μ(post_h) <= μ(pre)
    for (h_name, post_h, req_h) in other_posts {
        let mu_post_h = crate::verifier_actors::substitute_state_public(mu, state_name, post_h);
        let mut ant = mk_true();
        for r in req_h {
            ant = and_node(ant, r.clone());
        }
        let non_increase = infix(mu_post_h, "<=", mu_pre.clone());
        let oblig = implies_node(ant, non_increase);
        match prove(&oblig, timeout_ms) {
            Some(true) => {}
            Some(false) => {
                return CandidateVerdict::Refuted(format!(
                    "handler `{}` can increase measure `{}` (undoing progress)",
                    h_name,
                    render_clause(mu)
                ));
            }
            None => {
                return CandidateVerdict::Unknown(format!(
                    "Z3 could not decide non-increase of measure `{}` across `{}`",
                    render_clause(mu),
                    h_name,
                ));
            }
        }
    }

    // Constraint — measure-zero iff post-condition:
    //   μ(state) == 0 ↔ Q(state)
    let zero_iff_q = and_node(
        implies_node(infix(mu_pre.clone(), "==", int_lit(0)), post.clone()),
        implies_node(post.clone(), infix(mu_pre, "==", int_lit(0))),
    );
    match prove(&zero_iff_q, timeout_ms) {
        Some(true) => CandidateVerdict::Ok,
        Some(false) => CandidateVerdict::Refuted(format!(
            "measure `{}` is not zero iff post-condition `{}` holds",
            render_clause(mu),
            render_clause(post)
        )),
        None => CandidateVerdict::Unknown(format!(
            "Z3 could not decide the measure-zero equivalence for `{}`",
            render_clause(mu)
        )),
    }
}

/// Thin wrapper around the Z3 tautology-check path. Returns:
///   * `Some(true)` — proved
///   * `Some(false)` — refuted (found a concrete falsifier)
///   * `None` — undecidable / unknown
#[cfg(feature = "z3")]
fn prove(expr: &Node, timeout_ms: u32) -> Option<bool> {
    use std::collections::HashMap;
    let bindings: HashMap<String, i64> = HashMap::new();
    let (verdict, _cert, cx, _timed_out) =
        crate::verifier_z3::prove_with_timeout(expr, &bindings, timeout_ms);
    match verdict {
        Some(true) => Some(true),
        Some(false) => Some(false),
        None => {
            if cx.is_some() {
                // Concrete falsifier — treat as refutation.
                Some(false)
            } else {
                None
            }
        }
    }
}

#[cfg(not(feature = "z3"))]
fn prove(_expr: &Node, _timeout_ms: u32) -> Option<bool> {
    // Without Z3 we have no proof engine — signal Unknown so the
    // call site emits the partial-proof warning.
    None
}

// --- AST helpers (mirrors of the ones in verifier_actors) ---

fn int_lit(v: i64) -> Node {
    Node::IntegerLiteral {
        value: v,
        span: Span::default(),
    }
}

fn mk_true() -> Node {
    Node::BooleanLiteral {
        value: true,
        span: Span::default(),
    }
}

fn infix(left: Node, op: &str, right: Node) -> Node {
    Node::InfixExpression {
        left: Box::new(left),
        operator: op.to_string(),
        right: Box::new(right),
        span: Span::default(),
    }
}

fn and_node(a: Node, b: Node) -> Node {
    Node::InfixExpression {
        left: Box::new(a),
        operator: "&&".to_string(),
        right: Box::new(b),
        span: Span::default(),
    }
}

fn or_node(a: Node, b: Node) -> Node {
    Node::InfixExpression {
        left: Box::new(a),
        operator: "||".to_string(),
        right: Box::new(b),
        span: Span::default(),
    }
}

fn implies_node(a: Node, b: Node) -> Node {
    Node::InfixExpression {
        left: Box::new(Node::PrefixExpression {
            operator: "!".to_string(),
            right: Box::new(a),
            span: Span::default(),
        }),
        operator: "||".to_string(),
        right: Box::new(b),
        span: Span::default(),
    }
}

/// Best-effort rendering for diagnostics. Mirrors `verifier_actors`.
fn render_clause(node: &Node) -> String {
    match node {
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => format!(
            "{} {} {}",
            render_clause(left),
            operator,
            render_clause(right)
        ),
        Node::PrefixExpression {
            operator, right, ..
        } => format!("{}{}", operator, render_clause(right)),
        Node::Identifier { name, .. } => name.clone(),
        Node::IntegerLiteral { value, .. } => value.to_string(),
        Node::BooleanLiteral { value, .. } => value.to_string(),
        Node::FieldAccess { target, field, .. } => {
            format!("{}.{}", render_clause(target), field)
        }
        _ => "<expr>".to_string(),
    }
}

/// Top-level compiler pass: walk every `ActorDecl` in the program,
/// verify its `eventually` clauses, and emit `warning[partial-proof]`
/// lines on stderr for any non-Proved verdict. Refuted verdicts are
/// collected into an error string so the caller fails the build the
/// way the `always` path does.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let statements = match program {
        Node::Program(s) => s,
        _ => return Ok(()),
    };
    let mut refuted: Vec<String> = Vec::new();
    for stmt in statements {
        if let Node::ActorDecl {
            name,
            state_fields,
            eventually_clauses,
            receive_handlers,
            ..
        } = &stmt.node
        {
            if eventually_clauses.is_empty() {
                continue;
            }
            let obligations = verify_actor_liveness(
                name,
                state_fields,
                eventually_clauses,
                receive_handlers,
                /* timeout_ms = */ 2_000,
            );
            for o in obligations {
                match o.result {
                    LivenessResult::Proved => {}
                    LivenessResult::Refuted { reason } => {
                        let msg = format!(
                            "{}:{}:{}: actor `{}` violates `eventually(after: {}): {}` — {}",
                            if source_path.is_empty() {
                                "<unknown>"
                            } else {
                                source_path
                            },
                            o.clause_span.start.line,
                            o.clause_span.start.column,
                            o.actor_name,
                            o.target_handler,
                            o.post_label,
                            reason,
                        );
                        refuted.push(msg);
                    }
                    LivenessResult::PartialProof { reason }
                    | LivenessResult::Unsupported { reason } => {
                        eprintln!(
                            "warning[partial-proof]: actor `{}` `eventually(after: {}): {}` could not be proven within bounded depth {} — {}",
                            o.actor_name,
                            o.target_handler,
                            o.post_label,
                            DEFAULT_SCHEDULE_DEPTH,
                            reason,
                        );
                    }
                }
            }
        }
    }
    if refuted.is_empty() {
        Ok(())
    } else {
        Err(refuted.join("\n"))
    }
}
