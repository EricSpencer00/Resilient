// verifier_actors.rs
//
// RES-388: verify actor-level `always` safety invariants via Z3.
//
// For each `always: P;` clause on an `actor Name { ... }` we emit
// two proof obligations:
//
//   1. Base case. The state field's initializer E must satisfy P
//      — i.e. `P[state := E]` is a tautology.
//
//   2. Inductive step. For every `receive <h>(params) requires R
//      ensures _ { body }` handler, if P holds on the pre-state and
//      the handler's `requires` clauses hold, then P must hold on
//      the post-state. The post-state is computed by walking the
//      handler body and tracking the symbolic value of `state`
//      through each assignment. The obligation is:
//
//          (P ∧ R) → P[state := state_post]
//
// The body is required to be straight-line: a sequence of
// assignments (possibly nested inside a `Block`) of the form
//
//      self.state = <expr>;     -- FieldAssignment
//      state = <expr>;          -- Assignment
//
// Control flow (`if`, `while`, `for`) inside a handler body falls
// back to the "body not analyzable" diagnostic — intentionally
// scoped for MVP. Follow-ups extend the symbolic walk.
//
// When the compiler is built without `--features z3`, this module's
// public entry point is a no-op: the typechecker walk still runs
// (so syntax + type errors are caught) but no temporal proofs are
// attempted. This matches how `requires` / `ensures` behave today.

use crate::Node;
use crate::span::Span;
#[cfg(feature = "z3")]
use std::collections::HashMap;

/// Outcome of a per-clause proof attempt.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum ActorProofResult {
    /// Z3 discharged the obligation successfully.
    Proved,
    /// Z3 said the obligation does NOT hold — a counterexample
    /// assignment falsifies the post-invariant.
    Refuted { counterexample: Option<String> },
    /// Z3 returned `Unknown` (timeout / undecidable).
    Unknown,
    /// The handler body contains an unsupported construct that the
    /// symbolic walker can't translate. The `always` invariant is
    /// neither confirmed nor refuted; surfaced as a soft warning
    /// rather than a hard error so projects with mixed styles don't
    /// get blocked on a missing feature.
    Unsupported { reason: String },
}

/// One proof obligation produced for an actor.
#[derive(Debug, Clone)]
pub(crate) struct ActorObligation {
    pub actor_name: String,
    /// Either a handler name (`"enqueue"`) for inductive steps or
    /// the literal string `"<init>"` for the base case.
    pub handler_name: String,
    /// Human-readable rendering of the `always` clause — used in
    /// diagnostics so users know which invariant failed.
    pub invariant_label: String,
    pub invariant_span: Span,
    pub result: ActorProofResult,
}

/// Walk an actor declaration, emit a proof obligation per `always`
/// clause per handler, and return the verdict list. `timeout_ms`
/// threads the per-query Z3 timeout.
pub(crate) fn verify_actor(
    name: &str,
    state_fields: &[(String, String, Node)],
    always_clauses: &[Node],
    receive_handlers: &[crate::ReceiveHandler],
    timeout_ms: u32,
) -> Vec<ActorObligation> {
    let mut out = Vec::new();

    // MVP: single `state` field. Actors without state have nothing
    // to prove — skip. Multi-field support is a follow-up.
    let Some((_, state_name, state_init)) = state_fields.first() else {
        return out;
    };

    for clause in always_clauses {
        let label = render_clause(clause);
        let span = clause_span(clause);

        // --- Base case: P[state := init] ---
        let base = substitute_state(clause, state_name, state_init);
        out.push(ActorObligation {
            actor_name: name.to_string(),
            handler_name: "<init>".to_string(),
            invariant_label: label.clone(),
            invariant_span: span,
            result: prove_or_unsupported(&base, timeout_ms),
        });

        // --- Inductive step, one per handler. ---
        for h in receive_handlers {
            let Some(post_expr) = straight_line_post(&h.body, state_name) else {
                out.push(ActorObligation {
                    actor_name: name.to_string(),
                    handler_name: h.name.clone(),
                    invariant_label: label.clone(),
                    invariant_span: span,
                    result: ActorProofResult::Unsupported {
                        reason: format!(
                            "handler `{}` body is not straight-line — inductive step skipped",
                            h.name
                        ),
                    },
                });
                continue;
            };

            let post = substitute_state(clause, state_name, &post_expr);
            // (P_pre ∧ requires...) → P_post
            let mut antecedent = clause.clone();
            for r in &h.requires {
                antecedent = and_node(antecedent, r.clone());
            }
            let obligation = implies_node(antecedent, post);
            out.push(ActorObligation {
                actor_name: name.to_string(),
                handler_name: h.name.clone(),
                invariant_label: label.clone(),
                invariant_span: span,
                result: prove_or_unsupported(&obligation, timeout_ms),
            });
        }
    }

    out
}

/// RES-388 follow-up: public wrapper so the liveness verifier can
/// reuse the same state-substitution routine without duplicating it.
/// The module-private `substitute_state` stays the single source of
/// truth.
pub(crate) fn substitute_state_public(expr: &Node, state_name: &str, value: &Node) -> Node {
    substitute_state(expr, state_name, value)
}

/// RES-388 follow-up: public wrapper so the liveness verifier can
/// reuse the straight-line body walker. Returns `None` when the body
/// contains unsupported constructs, matching the private helper.
pub(crate) fn straight_line_post_public(body: &Node, state_name: &str) -> Option<Node> {
    straight_line_post(body, state_name)
}

/// Substitute every `Identifier(state_name)` and every
/// `FieldAccess { target = Identifier("self"), field = state_name }`
/// inside `expr` with `value`. Returns a fresh node; `expr` is
/// untouched.
fn substitute_state(expr: &Node, state_name: &str, value: &Node) -> Node {
    match expr {
        Node::Identifier { name, .. } if name == state_name => value.clone(),
        Node::FieldAccess { target, field, .. }
            if field == state_name
                && matches!(target.as_ref(), Node::Identifier { name, .. } if name == "self") =>
        {
            value.clone()
        }
        Node::InfixExpression {
            left,
            operator,
            right,
            span,
        } => Node::InfixExpression {
            left: Box::new(substitute_state(left, state_name, value)),
            operator: operator.clone(),
            right: Box::new(substitute_state(right, state_name, value)),
            span: *span,
        },
        Node::PrefixExpression {
            operator,
            right,
            span,
        } => Node::PrefixExpression {
            operator: operator.clone(),
            right: Box::new(substitute_state(right, state_name, value)),
            span: *span,
        },
        Node::CallExpression {
            function,
            arguments,
            span,
        } => Node::CallExpression {
            function: Box::new(substitute_state(function, state_name, value)),
            arguments: arguments
                .iter()
                .map(|a| substitute_state(a, state_name, value))
                .collect(),
            span: *span,
        },
        Node::FieldAccess {
            target,
            field,
            span,
        } => Node::FieldAccess {
            target: Box::new(substitute_state(target, state_name, value)),
            field: field.clone(),
            span: *span,
        },
        other => other.clone(),
    }
}

/// Walk a handler body assumed to be straight-line (a `Block` or a
/// single `FieldAssignment`/`Assignment`). Track the symbolic value
/// of `state_name` through each assignment step and return the
/// final symbolic expression, with every LHS substituted by its
/// preceding RHS so the result is expressed purely in terms of the
/// *pre-state* `state` plus handler parameters.
///
/// Returns `None` when the body contains any construct that isn't
/// an assignment to the state field — the caller treats this as
/// Unsupported.
fn straight_line_post(body: &Node, state_name: &str) -> Option<Node> {
    // Start with the symbolic pre-value: an `Identifier(state_name)`
    // so the verifier translates it to a free Z3 Int constant
    // representing the pre-state value of the field.
    let mut current = Node::Identifier {
        name: state_name.to_string(),
        span: Span::default(),
    };

    let stmts = flatten_body(body)?;
    for stmt in &stmts {
        match stmt {
            Node::FieldAssignment {
                target,
                field,
                value,
                ..
            } => {
                // Only track writes to `self.<state_name>`. Writes
                // to any other field are rejected for MVP — the
                // verifier doesn't model per-field state yet.
                if field != state_name
                    || !matches!(target.as_ref(), Node::Identifier { name, .. } if name == "self")
                {
                    return None;
                }
                current = substitute_state(value.as_ref(), state_name, &current);
            }
            Node::Assignment { name, value, .. } => {
                if name != state_name {
                    return None;
                }
                current = substitute_state(value.as_ref(), state_name, &current);
            }
            // Anything that isn't a plain assignment — reject.
            _ => return None,
        }
    }

    Some(current)
}

/// Unwrap nested `Block` nodes into a flat `Vec` of direct
/// statements. Returns `None` if anything other than an assignment
/// appears at a nesting level — callers treat that as Unsupported.
fn flatten_body(body: &Node) -> Option<Vec<Node>> {
    match body {
        Node::Block { stmts, .. } => {
            let mut out = Vec::new();
            for s in stmts {
                match s {
                    Node::Block { .. } => {
                        out.extend(flatten_body(s)?);
                    }
                    Node::FieldAssignment { .. } | Node::Assignment { .. } => {
                        out.push(s.clone());
                    }
                    // ExpressionStatement wrapping a pure expression
                    // could be allowed too, but we conservatively
                    // reject to keep MVP minimal.
                    _ => return None,
                }
            }
            Some(out)
        }
        Node::FieldAssignment { .. } | Node::Assignment { .. } => Some(vec![body.clone()]),
        _ => None,
    }
}

/// Build a logical-AND node over two operands.
fn and_node(a: Node, b: Node) -> Node {
    Node::InfixExpression {
        left: Box::new(a),
        operator: "&&".to_string(),
        right: Box::new(b),
        span: Span::default(),
    }
}

/// Build a logical-implication node, `a → b`, as `!a || b`.
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

/// Hand the obligation to Z3 (when the feature is on). Returns
/// `Unsupported` if the translation / verifier infrastructure isn't
/// available — keeping a build without `--features z3` green.
#[cfg(feature = "z3")]
fn prove_or_unsupported(expr: &Node, timeout_ms: u32) -> ActorProofResult {
    let bindings: HashMap<String, i64> = HashMap::new();
    let (verdict, _cert, cx, timed_out) =
        crate::verifier_z3::prove_with_timeout(expr, &bindings, timeout_ms);
    match verdict {
        Some(true) => ActorProofResult::Proved,
        // `Some(false)` means the formula itself is unsatisfiable —
        // a much stronger statement than "invariant broken". In our
        // encoding (`(P ∧ R) → P_post`) the formula being
        // unsatisfiable means there's *no* state at all that could
        // enter this handler — still a refutation of the temporal
        // claim from the user's perspective, so we surface it as
        // Refuted with whatever counterexample data Z3 produced.
        Some(false) => ActorProofResult::Refuted { counterexample: cx },
        None => {
            if timed_out {
                ActorProofResult::Unknown
            } else if cx.is_some() {
                // Not a tautology: Z3 found a concrete assignment
                // that falsifies the obligation. That is precisely
                // a refuted temporal claim — surface it as such
                // with the counterexample attached.
                ActorProofResult::Refuted { counterexample: cx }
            } else {
                // Neither a tautology nor a refutation — the
                // translator likely bailed (unsupported nodes,
                // floats, etc.). Fold into Unsupported.
                ActorProofResult::Unsupported {
                    reason:
                        "Z3 could not decide this obligation under the supported expression subset"
                            .to_string(),
                }
            }
        }
    }
}

#[cfg(not(feature = "z3"))]
fn prove_or_unsupported(_expr: &Node, _timeout_ms: u32) -> ActorProofResult {
    ActorProofResult::Unsupported {
        reason:
            "Z3 feature not enabled — rebuild with `--features z3` to verify `always` invariants"
                .to_string(),
    }
}

/// Best-effort rendering of an `always` clause for diagnostics. A
/// full pretty-printer lives in `formatter.rs`; this is the minimum
/// needed so users can distinguish which invariant a verdict is about.
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
        } => {
            format!("{}{}", operator, render_clause(right))
        }
        Node::Identifier { name, .. } => name.clone(),
        Node::IntegerLiteral { value, .. } => value.to_string(),
        Node::BooleanLiteral { value, .. } => value.to_string(),
        Node::FieldAccess { target, field, .. } => {
            format!("{}.{}", render_clause(target), field)
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let args: Vec<String> = arguments.iter().map(render_clause).collect();
            format!("{}({})", render_clause(function), args.join(", "))
        }
        _ => "<expr>".to_string(),
    }
}

fn clause_span(node: &Node) -> Span {
    match node {
        Node::IntegerLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::Identifier { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::CallExpression { span, .. }
        | Node::FieldAccess { span, .. } => *span,
        _ => Span::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(name: &str) -> Node {
        Node::Identifier {
            name: name.to_string(),
            span: Span::default(),
        }
    }

    fn int_lit(v: i64) -> Node {
        Node::IntegerLiteral {
            value: v,
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

    fn field(target: &str, name: &str) -> Node {
        Node::FieldAccess {
            target: Box::new(ident(target)),
            field: name.to_string(),
            span: Span::default(),
        }
    }

    fn field_assign(field_name: &str, rhs: Node) -> Node {
        Node::FieldAssignment {
            target: Box::new(ident("self")),
            field: field_name.to_string(),
            value: Box::new(rhs),
            span: Span::default(),
        }
    }

    fn block(stmts: Vec<Node>) -> Node {
        Node::Block {
            stmts,
            span: Span::default(),
        }
    }

    #[test]
    fn substitute_replaces_bare_identifier() {
        // `state <= 100`[state := 42]  ==>  `42 <= 100`.
        let clause = infix(ident("state"), "<=", int_lit(100));
        let replaced = substitute_state(&clause, "state", &int_lit(42));
        if let Node::InfixExpression { left, .. } = replaced {
            assert!(matches!(*left, Node::IntegerLiteral { value: 42, .. }));
        } else {
            panic!("expected InfixExpression");
        }
    }

    #[test]
    fn substitute_replaces_self_field_reference() {
        // `self.state + 1`[state := X]  ==>  `X + 1`.
        let clause = infix(field("self", "state"), "+", int_lit(1));
        let sub = ident("X");
        let replaced = substitute_state(&clause, "state", &sub);
        if let Node::InfixExpression { left, .. } = replaced {
            match *left {
                Node::Identifier { name, .. } => assert_eq!(name, "X"),
                other => panic!("expected replaced Identifier, got {:?}", other),
            }
        } else {
            panic!("expected InfixExpression");
        }
    }

    #[test]
    fn substitute_leaves_unrelated_identifiers_alone() {
        let clause = infix(ident("other"), ">", int_lit(0));
        let replaced = substitute_state(&clause, "state", &int_lit(99));
        if let Node::InfixExpression { left, .. } = replaced {
            match *left {
                Node::Identifier { name, .. } => assert_eq!(name, "other"),
                other => panic!("expected unchanged `other`, got {:?}", other),
            }
        } else {
            panic!("expected InfixExpression");
        }
    }

    #[test]
    fn straight_line_post_composes_self_field_assignments() {
        // Body:
        //   self.state = self.state + 1;
        //   self.state = self.state + 1;
        // Expected post = ((state + 1) + 1).
        let body = block(vec![
            field_assign("state", infix(field("self", "state"), "+", int_lit(1))),
            field_assign("state", infix(field("self", "state"), "+", int_lit(1))),
        ]);
        let post = straight_line_post(&body, "state").expect("straight-line post");
        // Shape: Infix(Infix(state, +, 1), +, 1)
        if let Node::InfixExpression { left, right, .. } = post {
            assert!(matches!(*right, Node::IntegerLiteral { value: 1, .. }));
            if let Node::InfixExpression {
                left: inner_left, ..
            } = *left
            {
                assert!(matches!(*inner_left, Node::Identifier { .. }));
            } else {
                panic!("expected nested Infix");
            }
        } else {
            panic!("expected top-level Infix");
        }
    }

    #[test]
    fn straight_line_post_rejects_non_state_assignment() {
        // A handler that writes to a field other than `state` isn't
        // analyzable by the MVP walker — it returns None.
        let body = block(vec![field_assign("other", int_lit(1))]);
        assert!(straight_line_post(&body, "state").is_none());
    }

    #[test]
    fn straight_line_post_rejects_control_flow() {
        // IfStatement inside the body — reject.
        let body = block(vec![Node::IfStatement {
            condition: Box::new(Node::BooleanLiteral {
                value: true,
                span: Span::default(),
            }),
            consequence: Box::new(block(vec![])),
            alternative: None,
            span: Span::default(),
        }]);
        assert!(straight_line_post(&body, "state").is_none());
    }

    #[test]
    fn render_clause_produces_readable_infix() {
        let clause = infix(ident("state"), "<=", int_lit(100));
        assert_eq!(render_clause(&clause), "state <= 100");
    }
}
