// verifier_z3.rs
//
// RES-067: Z3 SMT integration for the contract verifier.
//
// The hand-rolled folder (RES-060..065) handles a narrow but very
// useful subset of contract clauses. This module backstops it: when
// the folder returns Unknown, we hand the clause to Z3 and ask
// whether it's a tautology, a contradiction, or actually undecidable.
//
// The translation supports:
//   - integer literals
//   - identifiers (free or bound to a known integer in `bindings`)
//   - +, -, *, /, %  on integers
//   - ==, !=, <, >, <=, >=  comparisons
//   - !, &&, ||  logical connectives
//   - true, false
//
// Anything outside this subset (strings, arrays, structs, calls,
// floats) makes us bail to None — the existing runtime check still
// fires.

use crate::Node;
use std::collections::HashMap;
use z3::ast::{Ast, Bool, Int};

/// Return Some(true) if the expression is provably always true under
/// the bindings, Some(false) if provably always false, None if
/// undecidable or out of the supported subset.
pub fn prove(expr: &Node, bindings: &HashMap<String, i64>) -> Option<bool> {
    let cfg = z3::Config::new();
    let ctx = z3::Context::new(&cfg);

    // Translate the expression to a Z3 boolean.
    let formula = translate_bool(&ctx, expr, bindings)?;

    // Tautology check: is `NOT formula` unsatisfiable? If yes, formula
    // is always true regardless of any free variables.
    let solver = z3::Solver::new(&ctx);
    solver.assert(&formula.not());
    let tautology = matches!(solver.check(), z3::SatResult::Unsat);

    if tautology {
        return Some(true);
    }

    // Contradiction check: is `formula` unsatisfiable? If yes, the
    // contract can never hold.
    let solver = z3::Solver::new(&ctx);
    solver.assert(&formula);
    let contradiction = matches!(solver.check(), z3::SatResult::Unsat);

    if contradiction {
        return Some(false);
    }

    None
}

fn translate_bool<'c>(
    ctx: &'c z3::Context,
    node: &Node,
    bindings: &HashMap<String, i64>,
) -> Option<Bool<'c>> {
    match node {
        Node::BooleanLiteral(b) => Some(Bool::from_bool(ctx, *b)),
        Node::PrefixExpression { operator, right } if operator == "!" => {
            translate_bool(ctx, right, bindings).map(|b| b.not())
        }
        Node::InfixExpression { left, operator, right } => match operator.as_str() {
            "&&" => {
                let l = translate_bool(ctx, left, bindings)?;
                let r = translate_bool(ctx, right, bindings)?;
                Some(Bool::and(ctx, &[&l, &r]))
            }
            "||" => {
                let l = translate_bool(ctx, left, bindings)?;
                let r = translate_bool(ctx, right, bindings)?;
                Some(Bool::or(ctx, &[&l, &r]))
            }
            "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                let l = translate_int(ctx, left, bindings)?;
                let r = translate_int(ctx, right, bindings)?;
                let cmp = match operator.as_str() {
                    "==" => l._eq(&r),
                    "!=" => l._eq(&r).not(),
                    "<" => l.lt(&r),
                    ">" => l.gt(&r),
                    "<=" => l.le(&r),
                    ">=" => l.ge(&r),
                    _ => unreachable!(),
                };
                Some(cmp)
            }
            _ => None,
        },
        _ => None,
    }
}

fn translate_int<'c>(
    ctx: &'c z3::Context,
    node: &Node,
    bindings: &HashMap<String, i64>,
) -> Option<Int<'c>> {
    match node {
        Node::IntegerLiteral(v) => Some(Int::from_i64(ctx, *v)),
        Node::Identifier(name) => match bindings.get(name) {
            // If the name is bound to a known constant, model it as a
            // constant. Otherwise model it as a fresh free integer
            // variable so Z3 can reason about it universally.
            Some(v) => Some(Int::from_i64(ctx, *v)),
            None => Some(Int::new_const(ctx, name.as_str())),
        },
        Node::PrefixExpression { operator, right } if operator == "-" => {
            translate_int(ctx, right, bindings).map(|v| v.unary_minus())
        }
        Node::InfixExpression { left, operator, right } => {
            let l = translate_int(ctx, left, bindings)?;
            let r = translate_int(ctx, right, bindings)?;
            Some(match operator.as_str() {
                "+" => Int::add(ctx, &[&l, &r]),
                "-" => Int::sub(ctx, &[&l, &r]),
                "*" => Int::mul(ctx, &[&l, &r]),
                "/" => l.div(&r),
                "%" => l.rem(&r),
                _ => return None,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn z3_proves_tautology_no_bindings() {
        let no_b = HashMap::new();
        // `5 != 0` — provably true, no free variables.
        let expr = Node::InfixExpression {
            left: Box::new(Node::IntegerLiteral(5)),
            operator: "!=".to_string(),
            right: Box::new(Node::IntegerLiteral(0)),
        };
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn z3_proves_contradiction_no_bindings() {
        let no_b = HashMap::new();
        // `0 != 0` — provably false.
        let expr = Node::InfixExpression {
            left: Box::new(Node::IntegerLiteral(0)),
            operator: "!=".to_string(),
            right: Box::new(Node::IntegerLiteral(0)),
        };
        assert_eq!(prove(&expr, &no_b), Some(false));
    }

    #[test]
    fn z3_proves_universal_tautology_with_free_var() {
        // `x + 0 == x` is true for all x — the kind of thing the
        // hand-rolled folder CAN'T prove because x is free.
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::InfixExpression {
                left: Box::new(Node::Identifier("x".to_string())),
                operator: "+".to_string(),
                right: Box::new(Node::IntegerLiteral(0)),
            }),
            operator: "==".to_string(),
            right: Box::new(Node::Identifier("x".to_string())),
        };
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn z3_proves_implication_via_inequality() {
        // `x > 0 → x != 0`. We assert `x > 0 && !(x != 0)` — should
        // be unsat, meaning the implication holds. To frame it for
        // our prover: ask whether `(x > 0) || !(x > 0) || (x != 0)`
        // is a tautology. That's trivially true. A more interesting
        // case: prove `x * 2 > 0` from `x > 0`. We can't model the
        // implication directly with our prove() interface — instead
        // we build the combined formula as the input.
        // Simpler interesting case: `x > 0 || x <= 0` is a tautology.
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::InfixExpression {
                left: Box::new(Node::Identifier("x".to_string())),
                operator: ">".to_string(),
                right: Box::new(Node::IntegerLiteral(0)),
            }),
            operator: "||".to_string(),
            right: Box::new(Node::InfixExpression {
                left: Box::new(Node::Identifier("x".to_string())),
                operator: "<=".to_string(),
                right: Box::new(Node::IntegerLiteral(0)),
            }),
        };
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn z3_undecidable_returns_none_when_satisfiable() {
        // `x > 0` — neither tautology nor contradiction; Z3 returns
        // sat for both forms, so prove() returns None.
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::Identifier("x".to_string())),
            operator: ">".to_string(),
            right: Box::new(Node::IntegerLiteral(0)),
        };
        assert_eq!(prove(&expr, &no_b), None);
    }
}
