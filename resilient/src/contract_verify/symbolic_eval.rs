//! RES-3969 — body-aware `ensures` verification.
//!
//! The naive `ensures` proof asks Z3 whether `requires ⟹ ensures`
//! treating `result` as a *free* variable. Nothing ties `result` to
//! what the function actually returns, so a wrong `max` that always
//! returns `x` proves identically to a correct one — the obligation
//! `result >= x && result >= y` is simply satisfiable for *some*
//! `result`, and the free-variable query never rules that out.
//!
//! This module closes the hole for the straight-line and single
//! branch subset of function bodies by *substituting the body's
//! return expression for `result`* before the clause is proven:
//!
//! * **Straight-line** `{ return E; }` → prove `ensures[result := E]`.
//! * **Branching** `{ if C { return T; } else { return F; } }` (and the
//!   `if C { return T; } return F;` fall-through shape) → a case split:
//!   prove `ensures[result := T]` under the path condition `C`, and
//!   `ensures[result := F]` under `!C`. Both must hold.
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

use crate::Node;

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
        // { return E; }
        [only] => {
            let ret = return_value(only)?;
            is_pure(ret).then(|| ResultModel::Straight {
                ret: Box::new(ret.clone()),
            })
        }
        _ => None,
    }
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
