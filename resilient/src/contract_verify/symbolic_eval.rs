//! RES-3969 — body-aware `ensures` verification.
//!
//! The naive `ensures` proof asks Z3 whether `requires ⟹ ensures`
//! treating `result` as a *free* variable. Nothing ties `result` to
//! what the function actually returns, so a wrong `max` that always
//! returns `x` proves identically to a correct one — the obligation
//! `result >= x && result >= y` is simply satisfiable for *some*
//! `result`, and the free-variable query never rules that out.
//!
//! This module closes the hole for a conservative subset of function
//! bodies by *substituting the body's
//! return expression for `result`* before the clause is proven:
//!
//! * **Straight-line** `{ return E; }` → prove `ensures[result := E]`.
//! * **Branching** `{ if C { return T; } else { return F; } }` (and the
//!   `if C { return T; } return F;` fall-through shape) → a case split:
//!   prove `ensures[result := T]` under the path condition `C`, and
//!   `ensures[result := F]` under `!C`. Both must hold.
//! * **Match returns** with scalar literal / wildcard arms become a
//!   first-match case split.
//! * A single **try/catch** whose body and handlers each return a pure
//!   scalar expression becomes one possible return path per block.
//!
//! Path conditions ride the existing free-axiom channel
//! (`prove_with_axioms_and_timeout`): asserting `C` (or its negation)
//! as an axiom is exactly the antecedent of the per-branch
//! implication.
//!
//! Only *pure* return/condition expressions (identifiers, integer and
//! boolean literals, and prefix/infix operators over those) are
//! modelled. A call, field access, or any statement other than the
//! single `return` drops the body out of the subset and the caller
//! falls back to the labeled free-variable path.

use crate::{Node, Pattern};

/// How the return value of a function body relates to its inputs, for
/// the subset of bodies this pass can model exactly.
#[derive(Debug, Clone)]
pub(crate) enum ResultModel {
    /// `{ return E; }` — `result` is exactly `E`.
    Straight { ret: Box<Node> },
    /// `{ if C { return T; } else { return F; } }` or
    /// `{ if C { return T; } return F; }` — `result` is `T` when `C`
    /// holds and `F` otherwise.
    Branch {
        condition: Box<Node>,
        then_ret: Box<Node>,
        else_ret: Box<Node>,
    },
    /// A finite set of conservative return paths. `condition == None`
    /// represents an unconditional path, as used for a modeled
    /// `try` body or handler. Conditional paths account for first-match
    /// semantics before they reach the prover.
    Paths { paths: Vec<ResultPath> },
}

#[derive(Debug, Clone)]
pub(crate) struct ResultPath {
    pub(crate) condition: Option<Box<Node>>,
    pub(crate) ret: Box<Node>,
}

/// Model a function body's return value, or `None` when the body falls
/// outside the exactly-modelled subset.
pub(crate) fn model_body(body: &Node) -> Option<ResultModel> {
    let stmts = match body {
        Node::Block { stmts, .. } => stmts,
        _ => return None,
    };
    match stmts.as_slice() {
        // { if C { return T; } else { return F; } }
        [
            Node::IfStatement {
                condition,
                consequence,
                alternative: Some(alt),
                ..
            },
        ] => branch_model(condition, consequence, alt),
        // { if C { return T; } return F; }
        [
            Node::IfStatement {
                condition,
                consequence,
                alternative: None,
                ..
            },
            tail,
        ] => {
            let then_ret = single_return(consequence)?;
            let else_ret = return_value(tail)?;
            build_branch(condition, then_ret, else_ret)
        }
        // { try { return E; } catch Variant { return F; } ... }
        [Node::TryCatch { body, handlers, .. }] => try_model(body, handlers),
        // { return E; }
        [only] => {
            let ret = return_value(only)?;
            model_return_expression(ret)
        }
        _ => None,
    }
}

fn model_return_expression(expr: &Node) -> Option<ResultModel> {
    if is_pure(expr) {
        return Some(ResultModel::Straight {
            ret: Box::new(expr.clone()),
        });
    }
    match expr {
        Node::Match {
            scrutinee, arms, ..
        } => match_model(scrutinee, arms),
        _ => None,
    }
}

fn try_model(body: &[Node], handlers: &[(String, Vec<Node>)]) -> Option<ResultModel> {
    if handlers.is_empty() {
        return None;
    }
    let body_ret = return_from_statements(body)?;
    if !is_pure(body_ret) {
        return None;
    }
    let mut paths = vec![ResultPath {
        condition: None,
        ret: Box::new(body_ret.clone()),
    }];
    for (_, handler_body) in handlers {
        let ret = return_from_statements(handler_body)?;
        if !is_pure(ret) {
            return None;
        }
        paths.push(ResultPath {
            condition: None,
            ret: Box::new(ret.clone()),
        });
    }
    Some(ResultModel::Paths { paths })
}

fn return_from_statements(stmts: &[Node]) -> Option<&Node> {
    match stmts {
        [only] => return_value(only),
        _ => None,
    }
}

fn match_model(scrutinee: &Node, arms: &[(Pattern, Option<Node>, Node)]) -> Option<ResultModel> {
    if !is_pure(scrutinee) || arms.is_empty() {
        return None;
    }

    let mut covered = boolean_literal(false);
    let mut paths = Vec::with_capacity(arms.len());
    let mut has_unconditional_wildcard = false;

    for (pattern, guard, body) in arms {
        let ret = match_expression_value(body)?;
        if !is_pure(ret) {
            return None;
        }
        let arm_condition = pattern_condition(pattern, scrutinee)?;
        let arm_condition = if let Some(guard) = guard {
            if !is_pure(guard) {
                return None;
            }
            and(arm_condition, guard.clone())
        } else {
            if matches!(pattern, Pattern::Wildcard) {
                has_unconditional_wildcard = true;
            }
            arm_condition
        };
        let effective = and(arm_condition.clone(), negate(&covered));
        paths.push(ResultPath {
            condition: Some(Box::new(effective)),
            ret: Box::new(ret.clone()),
        });
        covered = or(covered, arm_condition);
    }

    // Without an unconditional wildcard, an unmatched scalar value can
    // produce Void. Refuse to model that partial result rather than
    // certifying only the listed arms.
    has_unconditional_wildcard.then_some(ResultModel::Paths { paths })
}

fn match_expression_value(node: &Node) -> Option<&Node> {
    if is_pure(node) {
        return Some(node);
    }
    let Node::Block { stmts, .. } = node else {
        return None;
    };
    match stmts.as_slice() {
        [Node::ExpressionStatement { expr, .. }] if is_pure(expr) => Some(expr),
        _ => None,
    }
}

fn pattern_condition(pattern: &Pattern, scrutinee: &Node) -> Option<Node> {
    match pattern {
        Pattern::Wildcard => Some(boolean_literal(true)),
        Pattern::Literal(literal)
            if matches!(
                literal,
                Node::IntegerLiteral { .. } | Node::BooleanLiteral { .. }
            ) =>
        {
            Some(infix(scrutinee.clone(), "==", literal.clone()))
        }
        _ => None,
    }
}

fn boolean_literal(value: bool) -> Node {
    Node::BooleanLiteral {
        value,
        span: crate::span::Span::default(),
    }
}

fn infix(left: Node, operator: &'static str, right: Node) -> Node {
    Node::InfixExpression {
        left: Box::new(left),
        operator,
        right: Box::new(right),
        span: crate::span::Span::default(),
    }
}

fn and(left: Node, right: Node) -> Node {
    infix(left, "&&", right)
}

fn or(left: Node, right: Node) -> Node {
    infix(left, "||", right)
}

fn branch_model(condition: &Node, consequence: &Node, alternative: &Node) -> Option<ResultModel> {
    let then_ret = single_return(consequence)?;
    let else_ret = single_return(alternative)?;
    build_branch(condition, then_ret, else_ret)
}

fn build_branch(condition: &Node, then_ret: &Node, else_ret: &Node) -> Option<ResultModel> {
    if is_pure(condition) && is_pure(then_ret) && is_pure(else_ret) {
        Some(ResultModel::Branch {
            condition: Box::new(condition.clone()),
            then_ret: Box::new(then_ret.clone()),
            else_ret: Box::new(else_ret.clone()),
        })
    } else {
        None
    }
}

/// The returned expression of a block that is exactly `{ return E; }`.
fn single_return(block: &Node) -> Option<&Node> {
    let Node::Block { stmts, .. } = block else {
        return None;
    };
    match stmts.as_slice() {
        [only] => return_value(only),
        _ => None,
    }
}

/// The value expression of a `return E;` statement (`None` for bare
/// `return;` or any non-return statement).
fn return_value(stmt: &Node) -> Option<&Node> {
    match stmt {
        Node::ReturnStatement { value: Some(v), .. } => Some(v),
        _ => None,
    }
}

/// A side-effect-free expression over the arithmetic/boolean subset the
/// Z3 translator models. Deliberately excludes calls and field access:
/// substituting a value that might have side effects or that the
/// translator treats as uninterpreted would produce an unsound
/// "implementation" verdict, so those bodies fall back to clause-only.
fn is_pure(expr: &Node) -> bool {
    match expr {
        Node::Identifier { .. } | Node::IntegerLiteral { .. } | Node::BooleanLiteral { .. } => true,
        Node::PrefixExpression { right, .. } => is_pure(right),
        Node::InfixExpression { left, right, .. } => is_pure(left) && is_pure(right),
        _ => false,
    }
}

/// Deep-clone `expr`, replacing every free occurrence of the contract
/// identifier `result` with `replacement`.
pub(crate) fn substitute_result(expr: &Node, replacement: &Node) -> Node {
    match expr {
        Node::Identifier { name, .. } if name == "result" => replacement.clone(),
        Node::PrefixExpression {
            operator,
            right,
            span,
        } => Node::PrefixExpression {
            operator,
            right: Box::new(substitute_result(right, replacement)),
            span: *span,
        },
        Node::InfixExpression {
            left,
            operator,
            right,
            span,
        } => Node::InfixExpression {
            left: Box::new(substitute_result(left, replacement)),
            operator,
            right: Box::new(substitute_result(right, replacement)),
            span: *span,
        },
        Node::CallExpression {
            function,
            arguments,
            span,
        } => Node::CallExpression {
            function: Box::new(substitute_result(function, replacement)),
            arguments: arguments
                .iter()
                .map(|a| substitute_result(a, replacement))
                .collect(),
            span: *span,
        },
        other => other.clone(),
    }
}

/// `!condition` — the else-branch path condition.
pub(crate) fn negate(condition: &Node) -> Node {
    Node::PrefixExpression {
        operator: "!",
        right: Box::new(condition.clone()),
        span: crate::span::Span::default(),
    }
}

/// Whether an `ensures` clause actually mentions `result`. A clause
/// that doesn't (`ensures x >= 0`, an input-only restatement) is
/// unaffected by substitution, so the two bases coincide and we keep
/// the clause-only label to avoid overclaiming.
pub(crate) fn mentions_result(expr: &Node) -> bool {
    match expr {
        Node::Identifier { name, .. } => name == "result",
        Node::PrefixExpression { right, .. } => mentions_result(right),
        Node::InfixExpression { left, right, .. } => {
            mentions_result(left) || mentions_result(right)
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => mentions_result(function) || arguments.iter().any(mentions_result),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn body_of(src: &str) -> Node {
        let (prog, _) = parse(src);
        let Node::Program(stmts) = prog else {
            panic!("not a program")
        };
        for s in stmts {
            if let Node::Function { body, .. } = s.node {
                return *body;
            }
        }
        panic!("no function in source")
    }

    #[test]
    fn models_straight_line_return() {
        let body = body_of("fn f(int x) -> int { return x; }");
        assert!(matches!(
            model_body(&body),
            Some(ResultModel::Straight { .. })
        ));
    }

    #[test]
    fn models_if_else_return() {
        let body =
            body_of("fn m(int x, int y) -> int { if x >= y { return x; } else { return y; } }");
        assert!(matches!(
            model_body(&body),
            Some(ResultModel::Branch { .. })
        ));
    }

    #[test]
    fn models_if_then_fallthrough_return() {
        let body = body_of("fn m(int x, int y) -> int { if x >= y { return x; } return y; }");
        assert!(matches!(
            model_body(&body),
            Some(ResultModel::Branch { .. })
        ));
    }

    #[test]
    fn models_scalar_match_return() {
        let body = body_of("fn m(int x) -> int { return match x { 0 => x, _ => x + 1 }; }");
        let Some(ResultModel::Paths { paths }) = model_body(&body) else {
            panic!("expected scalar match paths");
        };
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|path| path.condition.is_some()));
    }

    #[test]
    fn models_scalar_try_catch_returns() {
        let body = body_of("fn m(int x) -> int { try { return x; } catch Failure { return 0; } }");
        let Some(ResultModel::Paths { paths }) = model_body(&body) else {
            panic!("expected try/catch return paths");
        };
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|path| path.condition.is_none()));
    }

    #[test]
    fn rejects_multi_statement_body() {
        let body = body_of("fn f(int x) -> int { let t = x; return t; }");
        assert!(model_body(&body).is_none());
    }

    #[test]
    fn rejects_call_return_as_impure() {
        let body = body_of("fn f(int x) -> int { return g(x); }");
        assert!(model_body(&body).is_none());
    }

    #[test]
    fn substitutes_every_result_occurrence() {
        // ensures `result >= x && result >= y`, substitute result := x
        let clause = {
            let (prog, _) =
                parse("fn f(int x, int y) -> int ensures result >= x && result >= y { return x; }");
            let Node::Program(stmts) = prog else {
                unreachable!()
            };
            let mut c = None;
            for s in stmts {
                if let Node::Function { ensures, .. } = s.node {
                    c = ensures.into_iter().next();
                }
            }
            c.expect("ensures clause")
        };
        let x = Node::Identifier {
            name: "x".into(),
            span: crate::span::Span::default(),
        };
        let sub = substitute_result(&clause, &x);
        // No `result` identifier survives.
        assert!(!mentions_result(&sub));
    }

    #[test]
    fn negate_wraps_in_not() {
        let cond = Node::Identifier {
            name: "c".into(),
            span: crate::span::Span::default(),
        };
        assert!(matches!(
            negate(&cond),
            Node::PrefixExpression { operator: "!", .. }
        ));
    }
}
