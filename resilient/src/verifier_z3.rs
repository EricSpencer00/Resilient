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
use std::collections::{BTreeSet, HashMap};
use z3::ast::{Ast, Bool, Int};

/// RES-071: a re-verifiable SMT-LIB2 certificate captured when Z3
/// successfully discharges a contract obligation. Feeding the
/// `smt2` string to a stock Z3 (`z3 -smt2 cert.smt2`) must print
/// `unsat`, confirming the proof without trusting our binary.
#[derive(Debug, Clone)]
pub struct ProofCertificate {
    pub smt2: String,
}

/// Return Some(true) if the expression is provably always true under
/// the bindings, Some(false) if provably always false, None if
/// undecidable or out of the supported subset.
///
/// Thin wrapper over `prove_with_certificate` for callers that don't
/// need the SMT-LIB2 dump.
#[allow(dead_code)]
pub fn prove(expr: &Node, bindings: &HashMap<String, i64>) -> Option<bool> {
    prove_with_certificate(expr, bindings).0
}

/// RES-071: like `prove`, but ALSO returns a self-contained
/// SMT-LIB2 certificate when the verdict is `Some(true)`. The
/// certificate, fed to stock Z3, must print `unsat` — that is, the
/// negation of the contract clause is unsatisfiable, which is the
/// definition of a tautology proof. For `Some(false)` and `None`
/// verdicts the certificate is omitted.
pub fn prove_with_certificate(
    expr: &Node,
    bindings: &HashMap<String, i64>,
) -> (Option<bool>, Option<ProofCertificate>) {
    let (verdict, cert, _cx) = prove_with_certificate_and_counterexample(expr, bindings);
    (verdict, cert)
}

/// RES-136: full diagnostic version of `prove_with_certificate`.
/// Returns a third slot — a formatted counterexample — populated
/// when the negated formula is *satisfiable* (i.e. there is an
/// assignment that falsifies the clause). Callers that surface a
/// "could not prove" or "contract cannot hold" diagnostic to the
/// user can append this string to the error message.
///
/// The counterexample is `Some(...)` only when:
///   - The verdict is `Some(false)` (the clause is a contradiction —
///     any assignment falsifies it), OR
///   - The verdict is `None` (undecidable — at least one concrete
///     assignment was found to falsify the clause).
///
/// For `Some(true)` tautology proofs there is no counterexample and
/// the slot is `None`.
///
/// Format matches the ticket (`a = -1, b = 0`): identifier bindings
/// comma-separated, in deterministic BTreeSet order. Variables with
/// no assignment in the model are omitted — Z3 may elide them.
pub fn prove_with_certificate_and_counterexample(
    expr: &Node,
    bindings: &HashMap<String, i64>,
) -> (Option<bool>, Option<ProofCertificate>, Option<String>) {
    let cfg = z3::Config::new();
    let ctx = z3::Context::new(&cfg);

    // Translate the expression to a Z3 boolean.
    let formula = match translate_bool(&ctx, expr, bindings) {
        Some(f) => f,
        None => return (None, None, None),
    };

    // Tautology check: is `NOT formula` unsatisfiable? If yes, formula
    // is always true regardless of any free variables.
    let solver = z3::Solver::new(&ctx);
    let negated = formula.not();
    solver.assert(&negated);
    let check = solver.check();
    let tautology = matches!(check, z3::SatResult::Unsat);

    // RES-136: extract a counterexample whenever the negated formula
    // is satisfiable — the model is an assignment that falsifies the
    // clause, which is what a user needs to see to debug a failing
    // contract. We harvest it eagerly so the later contradiction
    // check (which reuses a fresh solver) doesn't need to re-derive
    // it.
    let counterexample = if matches!(check, z3::SatResult::Sat) {
        extract_counterexample(&ctx, &solver, expr, bindings)
    } else {
        None
    };

    if tautology {
        // Build a self-contained re-verifiable SMT-LIB2 file.
        // Strategy: declare every Int identifier that appears in the
        // expression, then constrain the bound ones to their concrete
        // value, then assert the NEGATED goal so a fresh Z3 returns
        // `unsat` (which is the proof that the original was always
        // true).
        let mut idents: BTreeSet<String> = BTreeSet::new();
        collect_int_identifiers(expr, &mut idents);

        let mut smt2 = String::new();
        smt2.push_str("; RES-071 verification certificate\n");
        smt2.push_str("; expected solver result: unsat (proves the contract is a tautology)\n");
        smt2.push_str("(set-logic AUFLIA)\n");
        for name in &idents {
            smt2.push_str(&format!("(declare-const {} Int)\n", name));
        }
        // Bound identifiers: pin them to their concrete value with an
        // equality assertion. Free identifiers are left unconstrained
        // so the proof is universal over them.
        for name in &idents {
            if let Some(v) = bindings.get(name) {
                smt2.push_str(&format!("(assert (= {} {}))\n", name, v));
            }
        }
        // The negated goal — Z3 ASTs Display as SMT-LIB2 syntax, so
        // we get a faithful round-trip via `negated.to_string()`.
        smt2.push_str(&format!("(assert {})\n", negated));
        smt2.push_str("(check-sat)\n");

        return (Some(true), Some(ProofCertificate { smt2 }), None);
    }

    // Contradiction check: is `formula` unsatisfiable? If yes, the
    // contract can never hold.
    let solver = z3::Solver::new(&ctx);
    solver.assert(&formula);
    let contradiction = matches!(solver.check(), z3::SatResult::Unsat);

    if contradiction {
        return (Some(false), None, counterexample);
    }

    (None, None, counterexample)
}

/// RES-136: harvest identifier assignments from a satisfied Z3
/// solver and format them as `name = value, name = value`. Only
/// integer identifiers the translator could produce are consulted.
///
/// We evaluate each identifier as an `Int` (the translator models
/// every free variable as `Int::new_const(name)`), request
/// model_completion=false so Z3 can legitimately return "not
/// constrained" — variables it didn't assign are silently dropped
/// per the ticket. Constants already pinned via `bindings` are also
/// dropped: echoing back input isn't useful diagnostic output.
fn extract_counterexample(
    ctx: &z3::Context,
    solver: &z3::Solver<'_>,
    expr: &Node,
    bindings: &HashMap<String, i64>,
) -> Option<String> {
    let model = solver.get_model()?;
    let mut idents: BTreeSet<String> = BTreeSet::new();
    collect_int_identifiers(expr, &mut idents);

    let mut parts: Vec<String> = Vec::new();
    for name in &idents {
        if bindings.contains_key(name) {
            continue;
        }
        let var = Int::new_const(ctx, name.as_str());
        if let Some(v) = model.eval(&var, false)
            && let Some(n) = v.as_i64()
        {
            parts.push(format!("{} = {}", name, n));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// Walk the AST collecting every identifier that the integer or boolean
/// translator could plausibly emit a `(declare-const NAME Int)` for.
/// Conservative — over-collecting is fine (extra unused declarations
/// don't change satisfiability); under-collecting would make the
/// certificate reference an undefined symbol and stock Z3 would error.
fn collect_int_identifiers(node: &Node, out: &mut BTreeSet<String>) {
    match node {
        Node::Identifier { name, .. } => {
            out.insert(name.clone());
        }
        Node::PrefixExpression { right, .. } => collect_int_identifiers(right, out),
        Node::InfixExpression { left, right, .. } => {
            collect_int_identifiers(left, out);
            collect_int_identifiers(right, out);
        }
        // Literals contribute no identifiers; everything else
        // (calls, blocks, etc.) is outside the supported subset and
        // would have caused translate_*() to bail already.
        _ => {}
    }
}

fn translate_bool<'c>(
    ctx: &'c z3::Context,
    node: &Node,
    bindings: &HashMap<String, i64>,
) -> Option<Bool<'c>> {
    match node {
        Node::BooleanLiteral { value: b, .. } => Some(Bool::from_bool(ctx, *b)),
        Node::PrefixExpression { operator, right, .. } if operator == "!" => {
            translate_bool(ctx, right, bindings).map(|b| b.not())
        }
        Node::InfixExpression { left, operator, right, .. } => match operator.as_str() {
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
        Node::IntegerLiteral { value: v, .. } => Some(Int::from_i64(ctx, *v)),
        Node::Identifier { name, .. } => match bindings.get(name) {
            // If the name is bound to a known constant, model it as a
            // constant. Otherwise model it as a fresh free integer
            // variable so Z3 can reason about it universally.
            Some(v) => Some(Int::from_i64(ctx, *v)),
            None => Some(Int::new_const(ctx, name.as_str())),
        },
        Node::PrefixExpression { operator, right, .. } if operator == "-" => {
            translate_int(ctx, right, bindings).map(|v| v.unary_minus())
        }
        Node::InfixExpression { left, operator, right, .. } => {
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
            left: Box::new(Node::IntegerLiteral { value: 5, span: crate::span::Span::default() }),
            operator: "!=".to_string(),
            right: Box::new(Node::IntegerLiteral { value: 0, span: crate::span::Span::default() }),
            span: crate::span::Span::default(),
        };
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn z3_proves_contradiction_no_bindings() {
        let no_b = HashMap::new();
        // `0 != 0` — provably false.
        let expr = Node::InfixExpression {
            left: Box::new(Node::IntegerLiteral { value: 0, span: crate::span::Span::default() }),
            operator: "!=".to_string(),
            right: Box::new(Node::IntegerLiteral { value: 0, span: crate::span::Span::default() }),
            span: crate::span::Span::default(),
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
                left: Box::new(Node::Identifier { name: "x".to_string(), span: crate::span::Span::default() }),
                operator: "+".to_string(),
                right: Box::new(Node::IntegerLiteral { value: 0, span: crate::span::Span::default() }),
            span: crate::span::Span::default(),
            }),
            operator: "==".to_string(),
            right: Box::new(Node::Identifier { name: "x".to_string(), span: crate::span::Span::default() }),
            span: crate::span::Span::default(),
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
                left: Box::new(Node::Identifier { name: "x".to_string(), span: crate::span::Span::default() }),
                operator: ">".to_string(),
                right: Box::new(Node::IntegerLiteral { value: 0, span: crate::span::Span::default() }),
            span: crate::span::Span::default(),
            }),
            operator: "||".to_string(),
            right: Box::new(Node::InfixExpression {
                left: Box::new(Node::Identifier { name: "x".to_string(), span: crate::span::Span::default() }),
                operator: "<=".to_string(),
                right: Box::new(Node::IntegerLiteral { value: 0, span: crate::span::Span::default() }),
            span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn certificate_for_tautology_contains_negated_goal_and_check_sat() {
        // RES-071: a successfully proven tautology yields a self-
        // contained .smt2 file declaring every free identifier and
        // asserting the negation of the goal.
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::InfixExpression {
                left: Box::new(Node::Identifier { name: "x".to_string(), span: crate::span::Span::default() }),
                operator: "+".to_string(),
                right: Box::new(Node::IntegerLiteral { value: 0, span: crate::span::Span::default() }),
            span: crate::span::Span::default(),
            }),
            operator: "==".to_string(),
            right: Box::new(Node::Identifier { name: "x".to_string(), span: crate::span::Span::default() }),
            span: crate::span::Span::default(),
        };
        let (verdict, cert) = prove_with_certificate(&expr, &no_b);
        assert_eq!(verdict, Some(true));
        let cert = cert.expect("tautology must yield a certificate");
        assert!(cert.smt2.contains("(declare-const x Int)"), "missing decl in:\n{}", cert.smt2);
        assert!(cert.smt2.contains("(check-sat)"), "missing check-sat in:\n{}", cert.smt2);
        assert!(cert.smt2.contains("(set-logic"), "missing set-logic in:\n{}", cert.smt2);
        assert!(cert.smt2.contains("(assert "), "missing negated assertion in:\n{}", cert.smt2);
    }

    #[test]
    fn certificate_pins_bound_identifiers_to_their_concrete_value() {
        // RES-071: when a parameter has a known constant binding, the
        // certificate must include an `(assert (= NAME VALUE))` so the
        // re-verification reflects the same call site.
        let mut bindings = HashMap::new();
        bindings.insert("n".to_string(), 5);
        let expr = Node::InfixExpression {
            left: Box::new(Node::Identifier { name: "n".to_string(), span: crate::span::Span::default() }),
            operator: ">".to_string(),
            right: Box::new(Node::IntegerLiteral { value: 0, span: crate::span::Span::default() }),
            span: crate::span::Span::default(),
        };
        let (verdict, cert) = prove_with_certificate(&expr, &bindings);
        assert_eq!(verdict, Some(true));
        let cert = cert.expect("bound tautology must yield a certificate");
        assert!(cert.smt2.contains("(declare-const n Int)"));
        assert!(cert.smt2.contains("(assert (= n 5))"), "missing binding pin:\n{}", cert.smt2);
    }

    #[test]
    fn certificate_is_omitted_for_undecidable() {
        // RES-071: don't emit a certificate when there's no proof.
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::Identifier { name: "x".to_string(), span: crate::span::Span::default() }),
            operator: ">".to_string(),
            right: Box::new(Node::IntegerLiteral { value: 0, span: crate::span::Span::default() }),
            span: crate::span::Span::default(),
        };
        let (_, cert) = prove_with_certificate(&expr, &no_b);
        assert!(cert.is_none(), "no proof => no cert");
    }

    #[test]
    fn z3_undecidable_returns_none_when_satisfiable() {
        // `x > 0` — neither tautology nor contradiction; Z3 returns
        // sat for both forms, so prove() returns None.
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::Identifier { name: "x".to_string(), span: crate::span::Span::default() }),
            operator: ">".to_string(),
            right: Box::new(Node::IntegerLiteral { value: 0, span: crate::span::Span::default() }),
            span: crate::span::Span::default(),
        };
        assert_eq!(prove(&expr, &no_b), None);
    }

    // --- RES-136: counterexample extraction from Z3 models ---

    /// Build `Node::Identifier { name }` with a default span.
    fn ident(name: &str) -> Node {
        Node::Identifier { name: name.to_string(), span: crate::span::Span::default() }
    }

    /// Build `Node::IntegerLiteral { value }` with a default span.
    fn int(value: i64) -> Node {
        Node::IntegerLiteral { value, span: crate::span::Span::default() }
    }

    /// Build `left OP right` with a default span.
    fn infix(left: Node, op: &str, right: Node) -> Node {
        Node::InfixExpression {
            left: Box::new(left),
            operator: op.to_string(),
            right: Box::new(right),
            span: crate::span::Span::default(),
        }
    }

    #[test]
    fn verifier_emits_counterexample_for_contradiction() {
        // `x > 5 && x < 0` — a strict contradiction. Verdict is
        // Some(false); counterexample is Z3's arbitrary witness to
        // the negation (anything not satisfying both conjuncts).
        let no_b = HashMap::new();
        let expr = infix(
            infix(ident("x"), ">", int(5)),
            "&&",
            infix(ident("x"), "<", int(0)),
        );
        let (verdict, _cert, cx) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, Some(false));
        let cx = cx.expect("contradiction must surface a counterexample");
        assert!(cx.contains("x ="), "counterexample should name `x`; got: {:?}", cx);
    }

    #[test]
    fn verifier_emits_counterexample_for_undecidable() {
        // `x > 0` — undecidable (neither always true nor always
        // false). Verdict is None; counterexample is a concrete
        // assignment where the clause fails — any x <= 0.
        let no_b = HashMap::new();
        let expr = infix(ident("x"), ">", int(0));
        let (verdict, _cert, cx) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, None);
        let cx = cx.expect("undecidable clause must surface a counterexample");
        assert!(cx.contains("x ="), "counterexample should name `x`; got: {:?}", cx);
    }

    #[test]
    fn verifier_omits_counterexample_for_tautology() {
        // `x + 0 == x` — tautology. No counterexample expected.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "+", int(0)), "==", ident("x"));
        let (verdict, _cert, cx) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, Some(true));
        assert!(cx.is_none(), "tautology should have no counterexample, got: {:?}", cx);
    }

    #[test]
    fn counterexample_omits_bound_identifiers() {
        // `n > 10` with `n` bound to 5: verdict is Some(false) and
        // the counterexample should NOT re-echo the pinned binding
        // (it's uninformative to print what the user already told us).
        let mut bindings = HashMap::new();
        bindings.insert("n".to_string(), 5);
        let expr = infix(ident("n"), ">", int(10));
        let (verdict, _cert, cx) = prove_with_certificate_and_counterexample(&expr, &bindings);
        assert_eq!(verdict, Some(false));
        // No free variables → no counterexample content.
        assert!(
            cx.as_deref().map(|s| !s.contains("n =")).unwrap_or(true),
            "bound identifier should not appear in counterexample: {:?}",
            cx,
        );
    }

    #[test]
    fn counterexample_names_multiple_free_identifiers() {
        // `a > 0 && b < 0` — undecidable.
        // Negation: `a <= 0 || b >= 0`. Z3 may only need to assign
        // one of the variables to satisfy a disjunction, so we
        // accept a counterexample that mentions at least one.
        let no_b = HashMap::new();
        let expr = infix(
            infix(ident("a"), ">", int(0)),
            "&&",
            infix(ident("b"), "<", int(0)),
        );
        let (verdict, _cert, cx) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, None);
        let cx = cx.expect("undecidable clause must surface a counterexample");
        assert!(
            cx.contains("a =") || cx.contains("b ="),
            "counterexample should name at least one free var; got: {:?}",
            cx,
        );
    }
}
