//! RES-390: distributed-invariant verifier.
//!
//! For each `cluster Name { ... cluster_invariant: EXPR; ... }`
//! declaration in a program, prove that `EXPR` is an **inductive**
//! invariant: if it holds before any `receive` handler of any
//! member actor fires, it still holds after. Z3 is the underlying
//! solver; we reuse `verifier_z3::prove_with_axioms_and_timeout`
//! so the translator, axiom handling, and counterexample harvest
//! are shared with the existing per-fn contract verifier.
//!
//! # Encoding
//!
//! Every member's state variable is a fresh Z3 `Int` identifier
//! named `<member>_<field>`. A handler `receive H() { ... }` is
//! symbolically executed against a *pre-state* binding (all
//! member.field = their own identifier). Each `self.field = EXPR`
//! assignment in the body rewrites the pre-state map, substituting
//! `self.field` uses with `<owning_member>_<field>` and any
//! `other_member.field` use with `<other_member>_<field>`. The
//! resulting expression becomes the post-state value of that field.
//! Fields not touched by the handler keep their pre-state identifier.
//!
//! The inductive obligation we hand to Z3 is then
//!
//! ```text
//! (invariant[pre]) implies (invariant[post])
//! ```
//!
//! — exactly the Hoare-style single-step preservation check the
//! ticket specifies. Z3 is asked to prove this as a tautology
//! over the joint state space; if it fails, the counterexample
//! (identifying a concrete pre-state assignment that breaks the
//! rule) is surfaced in the diagnostic alongside the offending
//! `actor` / `handler`.
//!
//! # Scope (MVP)
//!
//! - Only integer state fields — the joint Z3 model translates
//!   every member-field reference to a fresh `Int` const.
//! - Handler bodies restricted to a sequence of
//!   `self.field = EXPR;` assignments. Conditionals, loops,
//!   method calls, and message sends are follow-ups.
//! - No parameterised messages (`receive inc(x)` etc.) — needs
//!   existential quantification over payloads.
//! - No dynamic cluster membership, no inter-cluster communication,
//!   no liveness — out of scope per the ticket, filed as follow-ups.
//!
//! # RES-782: Network Partition and Dropped Delivery Modeling
//!
//! The MVP assumes a fully connected, reliable network: every message
//! sent is delivered. This is sound for verifying in the ideal case,
//! but many real distributed systems must tolerate network partitions
//! and dropped messages.
//!
//! Phase 1 (RES-782) extends the verifier surface to let users state
//! and check invariants under explicit partition / failure assumptions:
//!
//! - **Partition model**: A partition is a declaration of which members
//!   cannot communicate. For example, `partition { (A, B), (B, C) }`
//!   means A ↔ B cannot send/receive, B ↔ C cannot send/receive, but
//!   A ↔ C can. Messages sent across a partition edge are dropped.
//!
//! - **Dropped delivery**: A handler may not assume all sent messages
//!   arrive. For example, `send(other, msg)` in a partitioned context
//!   does not guarantee the receiver's mailbox is populated.
//!
//! - **Partition-resilient invariants**: An invariant can be checked
//!   under a specific partition assumption. A split-brain scenario that
//!   should violate the invariant will be rejected with a diagnostic.
//!   A partition-tolerant invariant remains provable under the same model.
//!
//! Example: a leader-election protocol should maintain "exactly one
//! leader in the connected set" within a partition. The verifier
//! should reject a scenario where two disjoint sets both elect leaders.

#[cfg(feature = "z3")]
use std::collections::HashMap;

#[cfg(feature = "z3")]
use crate::span::Span;
#[cfg(feature = "z3")]
use crate::{ActorHandler, Node};

// Helper: extract (field_name, init_expr) pairs from state_fields.
// The cluster verifier only needs field names + initializers (not type names).
#[cfg(feature = "z3")]
fn extract_state(state_fields: &[(String, String, Node)]) -> Vec<(String, Node)> {
    state_fields
        .iter()
        .map(|(_, n, v)| (n.clone(), v.clone()))
        .collect()
}

/// RES-390: per-actor spec — (state fields, handlers) — used by the
/// cluster verifier to resolve `member: ActorType` into its handler
/// list. Extracted into a type alias so clippy stops yelling about
/// the nested `HashMap<String, (Vec<...>, Vec<...>)>` shape.
#[cfg(feature = "z3")]
type ActorTable = HashMap<String, (Vec<(String, Node)>, Vec<ActorHandler>)>;

/// RES-390: one diagnostic produced by the cluster verifier.
/// `actor` and `handler` identify the member whose handler broke
/// the invariant, `message` is the human-readable explanation, and
/// `span` points at the `cluster_invariant` expression (so the
/// caret underline lands on the invariant the user wrote, not on
/// the handler body — matching how `ensures` diagnostics surface).
#[cfg(feature = "z3")]
#[derive(Debug, Clone)]
pub(crate) struct ClusterDiagnostic {
    pub(crate) cluster: String,
    pub(crate) actor: String,
    pub(crate) handler: String,
    pub(crate) message: String,
    pub(crate) span: Span,
}

/// RES-390: verify every cluster in `program`. Returns the full
/// list of diagnostics for unproven invariants; an empty `Vec`
/// means every cluster invariant is inductively preserved.
///
/// `timeout_ms` is plumbed through to the Z3 solver. `0` disables
/// the timeout.
#[cfg(feature = "z3")]
pub(crate) fn verify_program(program: &Node, timeout_ms: u32) -> Vec<ClusterDiagnostic> {
    let Node::Program(stmts) = program else {
        return Vec::new();
    };

    // Build an `actor name → &ActorDecl-fields` lookup so the
    // cluster verifier can resolve member types.
    let mut actors: ActorTable = HashMap::new();
    for spanned in stmts {
        if let Node::ActorDecl {
            name,
            state_fields,
            handlers,
            ..
        } = &spanned.node
        {
            actors.insert(
                name.clone(),
                (extract_state(state_fields), handlers.clone()),
            );
        }
    }

    let mut out = Vec::new();
    for spanned in stmts {
        if let Node::ClusterDecl {
            name,
            members,
            invariants,
            span,
        } = &spanned.node
        {
            verify_cluster(
                name, members, invariants, *span, &actors, timeout_ms, &mut out,
            );
        }
    }
    out
}

/// Verify one cluster: for each (member, handler) pair prove that
/// the invariant is preserved. Push any failures to `out`.
#[cfg(feature = "z3")]
fn verify_cluster(
    cluster_name: &str,
    members: &[(String, String)],
    invariants: &[Node],
    cluster_span: Span,
    actors: &ActorTable,
    timeout_ms: u32,
    out: &mut Vec<ClusterDiagnostic>,
) {
    // Sanity: each member must reference a declared actor. Missing
    // actors surface one diagnostic per invariant so the user sees
    // a concrete cause even without counterexample data.
    for (local, actor_ty) in members {
        if !actors.contains_key(actor_ty) {
            for inv in invariants {
                out.push(ClusterDiagnostic {
                    cluster: cluster_name.to_string(),
                    actor: local.clone(),
                    handler: "<missing>".to_string(),
                    message: format!(
                        "cluster `{}` references actor type `{}` which is not declared — \
                         declare `actor {} {{ ... }}` before the cluster, or fix the typo",
                        cluster_name, actor_ty, actor_ty
                    ),
                    span: inv_span_or_cluster(inv, cluster_span),
                });
            }
            return;
        }
    }

    for inv in invariants {
        // Skip invariants the translator can't express as booleans —
        // emit a diagnostic so the user knows it's not being proven,
        // rather than silently passing an unchecked invariant.
        if !is_supported_invariant(inv) {
            out.push(ClusterDiagnostic {
                cluster: cluster_name.to_string(),
                actor: "<any>".to_string(),
                handler: "<static>".to_string(),
                message: format!(
                    "cluster `{}` invariant uses a construct outside the Z3 integer subset — \
                     supported today: integer literals, `member.field`, +, -, *, /, %, \
                     ==, !=, <, >, <=, >=, !, &&, ||",
                    cluster_name
                ),
                span: inv_span_or_cluster(inv, cluster_span),
            });
            continue;
        }

        for (local, actor_ty) in members {
            let Some((_state, handlers)) = actors.get(actor_ty) else {
                continue;
            };
            for handler in handlers {
                if let Some(diag) = verify_handler_against_invariant(
                    cluster_name,
                    local,
                    members,
                    handler,
                    inv,
                    cluster_span,
                    timeout_ms,
                ) {
                    out.push(diag);
                }
            }
        }
    }
}

/// Collect every `member.field` pair referenced in `node`. Used to
/// sanity-check the invariant expression before we hand it to Z3.
#[cfg(feature = "z3")]
fn is_supported_invariant(node: &Node) -> bool {
    match node {
        Node::BooleanLiteral { .. } | Node::IntegerLiteral { .. } => true,
        Node::Identifier { .. } => true,
        Node::FieldAccess { target, .. } => matches!(target.as_ref(), Node::Identifier { .. }),
        Node::PrefixExpression {
            right, operator, ..
        } => matches!(operator.as_str(), "!" | "-") && is_supported_invariant(right),
        Node::InfixExpression {
            left,
            right,
            operator,
            ..
        } => {
            matches!(
                operator.as_str(),
                "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||"
            ) && is_supported_invariant(left)
                && is_supported_invariant(right)
        }
        _ => false,
    }
}

/// Verify that one handler preserves the invariant. Returns
/// `Some(diagnostic)` if Z3 can't prove inductive preservation;
/// `None` on success (proof found) or on a translate-failure in
/// the pre-/post-state expressions (which we treat as "can't
/// prove" and emit a pointed diagnostic so the user doesn't get
/// a silent pass).
#[cfg(feature = "z3")]
fn verify_handler_against_invariant(
    cluster_name: &str,
    owning_member: &str,
    members: &[(String, String)],
    handler: &ActorHandler,
    invariant: &Node,
    cluster_span: Span,
    timeout_ms: u32,
) -> Option<ClusterDiagnostic> {
    // Pre-state: every member.field becomes a fresh Z3 Int.
    // We DON'T pin bindings — leaving them unbound makes the
    // Z3 consts universal, which is what "for all pre-states
    // satisfying the invariant …" requires.
    let pre_invariant = match rewrite_field_refs(invariant, /* post = */ None) {
        Some(n) => n,
        None => {
            return Some(translate_failure(
                cluster_name,
                owning_member,
                handler,
                cluster_span,
            ));
        }
    };

    // Collect assignments `self.field = EXPR;` from the handler
    // body. If the body contains anything else, record a
    // diagnostic that tells the user what to strip so the
    // verifier can see the handler's effect.
    let assignments = match collect_self_assignments(&handler.body) {
        Ok(asgs) => asgs,
        Err(msg) => {
            return Some(ClusterDiagnostic {
                cluster: cluster_name.to_string(),
                actor: owning_member.to_string(),
                handler: handler.name.clone(),
                message: format!(
                    "cluster `{}`: handler `{}.{}` uses `{}` — \
                     the RES-390 MVP verifier supports only a sequence of \
                     `self.field = EXPR;` assignments. File a follow-up \
                     if you need richer handler bodies.",
                    cluster_name, owning_member, handler.name, msg
                ),
                span: handler.span,
            });
        }
    };

    // Build the post-state rewrite for the owning member's fields.
    // Fields not touched by this handler default to their pre-state
    // identifier. Other members' fields always keep their pre-state
    // identifier — the handler only mutates `self`.
    let mut post_rewrites: HashMap<String, Node> = HashMap::new();
    for (field, rhs) in &assignments {
        let rewritten = match rewrite_field_refs(rhs, Some(owning_member)) {
            Some(n) => n,
            None => {
                return Some(translate_failure(
                    cluster_name,
                    owning_member,
                    handler,
                    cluster_span,
                ));
            }
        };
        post_rewrites.insert(field.clone(), rewritten);
    }

    // Rewrite the invariant with `owning_member.field` substituted
    // for its post-state expression (from `post_rewrites`). Other
    // `member.field` references keep their pre-state identifier.
    let post_invariant =
        match rewrite_invariant_post(invariant, owning_member, members, &post_rewrites) {
            Some(n) => n,
            None => {
                return Some(translate_failure(
                    cluster_name,
                    owning_member,
                    handler,
                    cluster_span,
                ));
            }
        };

    // Obligation: `pre_invariant → post_invariant` must be a
    // tautology over the joint state. We hand this to Z3 as a
    // single implication and check for tautology. The Z3
    // translator understands `!`, `&&`, `||`, and all the
    // comparison / arithmetic operators the subset covers — so
    // implication gets desugared as `!pre || post`, keeping us
    // inside the supported grammar.
    let implication = Node::InfixExpression {
        left: Box::new(Node::PrefixExpression {
            operator: "!".to_string(),
            right: Box::new(pre_invariant.clone()),
            span: Span::default(),
        }),
        operator: "||".to_string(),
        right: Box::new(post_invariant.clone()),
        span: Span::default(),
    };

    // Empty bindings — all member.field identifiers are universal
    // free vars the translator models as `Int::new_const(...)`.
    // We add `pre_invariant` itself as an axiom so the solver
    // restricts to the pre-states that satisfy the invariant
    // (otherwise Z3 trivially finds counterexamples in states
    // that already violated the invariant before the handler ran —
    // those aren't real failures of the handler).
    let bindings: HashMap<String, i64> = HashMap::new();
    let axioms = vec![pre_invariant.clone()];
    let (verdict, _cert, counterexample, timed_out) =
        crate::verifier_z3::prove_with_axioms_and_timeout(
            &implication,
            &bindings,
            &axioms,
            timeout_ms,
        );

    match verdict {
        Some(true) => None,
        Some(false) => Some(ClusterDiagnostic {
            cluster: cluster_name.to_string(),
            actor: owning_member.to_string(),
            handler: handler.name.clone(),
            message: format_cex_message(cluster_name, owning_member, handler, counterexample),
            span: cluster_span,
        }),
        None => {
            let hint = if timed_out {
                " (Z3 timed out — try a higher `--verifier-timeout-ms`)"
            } else {
                ""
            };
            Some(ClusterDiagnostic {
                cluster: cluster_name.to_string(),
                actor: owning_member.to_string(),
                handler: handler.name.clone(),
                message: format!(
                    "cluster `{}` invariant could not be proved inductive for \
                     `{}.{}`{}: {}",
                    cluster_name,
                    owning_member,
                    handler.name,
                    hint,
                    counterexample
                        .as_deref()
                        .unwrap_or("no counterexample available"),
                ),
                span: cluster_span,
            })
        }
    }
}

#[cfg(feature = "z3")]
fn translate_failure(
    cluster_name: &str,
    owning_member: &str,
    handler: &ActorHandler,
    cluster_span: Span,
) -> ClusterDiagnostic {
    ClusterDiagnostic {
        cluster: cluster_name.to_string(),
        actor: owning_member.to_string(),
        handler: handler.name.clone(),
        message: format!(
            "cluster `{}`: unable to translate invariant or handler body to Z3 for \
             `{}.{}` — the expression uses a construct outside the supported subset",
            cluster_name, owning_member, handler.name
        ),
        span: cluster_span,
    }
}

#[cfg(feature = "z3")]
fn format_cex_message(
    cluster_name: &str,
    owning_member: &str,
    handler: &ActorHandler,
    counterexample: Option<String>,
) -> String {
    let mut msg = format!(
        "cluster `{}` invariant broken by `{}.{}`",
        cluster_name, owning_member, handler.name
    );
    if let Some(cx) = counterexample {
        msg.push_str(" — pre-state counterexample: ");
        msg.push_str(&cx);
    }
    msg
}

/// Rewrite every `member.field` access in `node` to a plain
/// `Identifier("<member>_<field>")`. When `scope_as_self` is
/// `Some(owner)`, bare `self.field` accesses are rewritten as
/// `<owner>_<field>`. Returns `None` if we encounter a shape we
/// don't know how to translate.
#[cfg(feature = "z3")]
fn rewrite_field_refs(node: &Node, scope_as_self: Option<&str>) -> Option<Node> {
    match node {
        Node::BooleanLiteral { .. } | Node::IntegerLiteral { .. } => Some(node.clone()),
        Node::Identifier { .. } => Some(node.clone()),
        Node::FieldAccess {
            target,
            field,
            span,
        } => {
            let owner = match target.as_ref() {
                Node::Identifier { name, .. } => {
                    if name == "self" {
                        scope_as_self?.to_string()
                    } else {
                        name.clone()
                    }
                }
                _ => return None,
            };
            Some(Node::Identifier {
                name: format!("{}_{}", owner, field),
                span: *span,
            })
        }
        Node::PrefixExpression {
            operator,
            right,
            span,
        } => Some(Node::PrefixExpression {
            operator: operator.clone(),
            right: Box::new(rewrite_field_refs(right, scope_as_self)?),
            span: *span,
        }),
        Node::InfixExpression {
            left,
            operator,
            right,
            span,
        } => Some(Node::InfixExpression {
            left: Box::new(rewrite_field_refs(left, scope_as_self)?),
            operator: operator.clone(),
            right: Box::new(rewrite_field_refs(right, scope_as_self)?),
            span: *span,
        }),
        _ => None,
    }
}

/// Rewrite the invariant for the post-state: every
/// `owning_member.field` reference is substituted with its
/// post-state expression (from `post_rewrites`), falling back
/// to the pre-state identifier when the handler didn't touch
/// that field. Other members' fields keep their pre-state
/// identifier (they are unaffected by the handler).
#[cfg(feature = "z3")]
fn rewrite_invariant_post(
    node: &Node,
    owning_member: &str,
    members: &[(String, String)],
    post_rewrites: &HashMap<String, Node>,
) -> Option<Node> {
    match node {
        Node::BooleanLiteral { .. } | Node::IntegerLiteral { .. } => Some(node.clone()),
        Node::Identifier { .. } => Some(node.clone()),
        Node::FieldAccess {
            target,
            field,
            span,
        } => {
            let Node::Identifier { name: mname, .. } = target.as_ref() else {
                return None;
            };
            let is_member = members.iter().any(|(local, _)| local == mname);
            if !is_member {
                return None;
            }
            if mname == owning_member
                && let Some(rhs) = post_rewrites.get(field)
            {
                return Some(rhs.clone());
            }
            Some(Node::Identifier {
                name: format!("{}_{}", mname, field),
                span: *span,
            })
        }
        Node::PrefixExpression {
            operator,
            right,
            span,
        } => Some(Node::PrefixExpression {
            operator: operator.clone(),
            right: Box::new(rewrite_invariant_post(
                right,
                owning_member,
                members,
                post_rewrites,
            )?),
            span: *span,
        }),
        Node::InfixExpression {
            left,
            operator,
            right,
            span,
        } => Some(Node::InfixExpression {
            left: Box::new(rewrite_invariant_post(
                left,
                owning_member,
                members,
                post_rewrites,
            )?),
            operator: operator.clone(),
            right: Box::new(rewrite_invariant_post(
                right,
                owning_member,
                members,
                post_rewrites,
            )?),
            span: *span,
        }),
        _ => None,
    }
}

/// Walk a handler body and collect every `self.field = EXPR;`
/// assignment, in source order. Any other statement shape is an
/// `Err(human-readable-descriptor)` so the caller can diagnose.
///
/// Multiple assignments to the same field keep only the last —
/// matching straight-line execution semantics. (A later
/// ticket can model each assignment's pre-state independently
/// to support `x = y; y = x;` swaps, but the single-rewrite-per-
/// field rule is sound today for the single-leader MVP.)
#[cfg(feature = "z3")]
fn collect_self_assignments(body: &Node) -> Result<Vec<(String, Node)>, String> {
    let Node::Block { stmts, .. } = body else {
        return Err(format!(
            "handler body is {}, expected `{{ ... }}`",
            node_kind(body)
        ));
    };
    let mut out: Vec<(String, Node)> = Vec::new();
    for stmt in stmts {
        let Node::FieldAssignment {
            target,
            field,
            value,
            ..
        } = stmt
        else {
            return Err(format!(
                "statement `{}` — supported: `self.<field> = EXPR;`",
                node_kind(stmt)
            ));
        };
        match target.as_ref() {
            Node::Identifier { name, .. } if name == "self" => {}
            _ => {
                return Err(format!(
                    "assignment target is {}, expected `self.<field>`",
                    node_kind(target)
                ));
            }
        }
        // Replace any earlier write to this field so the last
        // assignment wins, then append so source order is
        // preserved for the unmodified fields. `retain` +
        // `push` rather than a map keeps the Vec ordering.
        out.retain(|(f, _)| f != field);
        out.push((field.clone(), (**value).clone()));
    }
    Ok(out)
}

/// Human-readable node kind for diagnostics. Matches the terms
/// that appear in the RES-390 scope notes so the error message
/// reads like the feature description.
#[cfg(feature = "z3")]
fn node_kind(n: &Node) -> &'static str {
    match n {
        Node::Block { .. } => "block",
        Node::LetStatement { .. } => "`let` binding",
        Node::StaticLet { .. } => "`static let` binding",
        Node::Assignment { .. } => "local assignment",
        Node::FieldAssignment { .. } => "field assignment",
        Node::IndexAssignment { .. } => "index assignment",
        Node::IfStatement { .. } => "`if` statement",
        Node::WhileStatement { .. } => "`while` loop",
        Node::ForInStatement { .. } => "`for`-in loop",
        Node::CallExpression { .. } => "function call",
        Node::ReturnStatement { .. } => "`return` statement",
        Node::ExpressionStatement { .. } => "expression statement",
        Node::Match { .. } => "`match` expression",
        Node::LiveBlock { .. } => "`live` block",
        Node::Assert { .. } => "`assert`",
        Node::Assume { .. } => "`assume`",
        Node::TryExpression { .. } => "`?` operator",
        Node::OptionalChain { .. } => "`?.` operator",
        Node::Identifier { .. } => "identifier",
        Node::FieldAccess { .. } => "field access",
        _ => "unsupported construct",
    }
}

#[cfg(feature = "z3")]
fn inv_span_or_cluster(inv: &Node, fallback: Span) -> Span {
    match inv {
        Node::InfixExpression { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::Identifier { span, .. }
        | Node::IntegerLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::FieldAccess { span, .. } => *span,
        _ => fallback,
    }
}

#[cfg(all(test, feature = "z3"))]
mod tests {
    use super::*;
    use crate::parse;

    fn verify(src: &str) -> Vec<ClusterDiagnostic> {
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        verify_program(&program, 5000)
    }

    #[test]
    fn single_leader_cluster_with_only_step_down_passes() {
        // Invariant: at most one leader; the `state` flag is 0 or 1.
        // Only a `step_down` handler exists — it assigns
        // `state = 0`, which can only decrease the sum and keeps
        // the per-field [0, 1] range, so Z3 proves preservation.
        // The range clauses are load-bearing: without them Z3
        // finds a pre-state like `a = -1, b = 2` that satisfies
        // `a + b <= 1` but breaks after zeroing `a`.
        let src = r#"
actor Node {
    state: int = 0;

    receive step_down() {
        self.state = 0;
    }
}

cluster Ring {
    a: Node;
    b: Node;
    cluster_invariant: a.state >= 0 && a.state <= 1
                    && b.state >= 0 && b.state <= 1
                    && a.state + b.state <= 1;
}
"#;
        let diags = verify(src);
        assert!(diags.is_empty(), "expected no diagnostics, got {:?}", diags);
    }

    #[test]
    fn single_leader_cluster_with_become_leader_fails() {
        // A `become_leader` handler sets `state = 1` on the caller
        // without coordinating with the other replica — Z3 should
        // find a pre-state (the peer is already leader) where the
        // post-state violates the invariant.
        let src = r#"
actor Node {
    state: int = 0;

    receive become_leader() {
        self.state = 1;
    }
}

cluster Ring {
    a: Node;
    b: Node;
    cluster_invariant: a.state >= 0 && a.state <= 1
                    && b.state >= 0 && b.state <= 1
                    && a.state + b.state <= 1;
}
"#;
        let diags = verify(src);
        assert!(!diags.is_empty(), "expected at least one diagnostic");
        assert!(
            diags.iter().any(|d| d.handler == "become_leader"),
            "expected a diagnostic pointing at `become_leader`, got {:?}",
            diags
        );
    }

    #[test]
    fn missing_actor_type_reports_diagnostic() {
        let src = r#"
cluster Broken {
    a: MissingActor;
    cluster_invariant: a.x <= 1;
}
"#;
        let diags = verify(src);
        assert!(!diags.is_empty());
        assert!(
            diags[0].message.contains("not declared"),
            "expected missing-actor message, got {:?}",
            diags[0]
        );
    }

    #[test]
    fn unsupported_handler_shape_reports_diagnostic() {
        // `if` in a handler body is not supported by the MVP
        // symbolic executor — we must diagnose rather than
        // silently pass.
        let src = r#"
actor Toggle {
    state: int = 0;

    receive flip() {
        if state == 0 { self.state = 1; } else { self.state = 0; }
    }
}

cluster Single {
    t: Toggle;
    cluster_invariant: t.state <= 1;
}
"#;
        let diags = verify(src);
        assert!(!diags.is_empty(), "expected a diagnostic");
        assert!(
            diags.iter().any(|d| d.message.contains("MVP")),
            "expected MVP-scope message, got {:?}",
            diags
        );
    }
}
