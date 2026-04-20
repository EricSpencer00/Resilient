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
    let (v, c, cx, _timed_out) = prove_with_timeout(expr, bindings, 0);
    (v, c, cx)
}

/// RES-137: like `prove_with_certificate_and_counterexample` but
/// with a per-query wall-clock timeout in milliseconds. A value of
/// 0 disables the timeout (use the solver's default, which is
/// unlimited).
///
/// The fourth return slot is `true` when Z3 reported `Unknown` —
/// i.e. the tautology check timed out. Callers treat this the same
/// as the existing `None` verdict (not proven → runtime check
/// retained) but get enough signal to emit a hint diagnostic and
/// to bump the `timed-out` audit counter.
pub fn prove_with_timeout(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    timeout_ms: u32,
) -> (Option<bool>, Option<ProofCertificate>, Option<String>, bool) {
    prove_with_axioms_and_timeout(expr, bindings, &[], timeout_ms)
}

/// FFI Phase 1 Task 10: prove `expr` under an additional list of
/// free boolean `axioms` that the solver treats as `true`. Designed
/// for `@trusted` extern fn `ensures` clauses: a caller collects the
/// trusted ensures that reference values in scope, rewrites them so
/// every occurrence of `result` is replaced with the call site's
/// return-value identifier, and hands the list to this function as
/// axioms.
///
/// Axioms that fail to translate (unsupported nodes, floats, etc.)
/// are silently skipped — the same fail-open policy the rest of the
/// translator uses. A silently skipped axiom is safe: dropping
/// information can only weaken the assumption set, never make an
/// unsound verdict sound.
///
/// Return shape matches `prove_with_timeout`. The
/// certificate-generation path does NOT yet embed the axioms in the
/// emitted SMT-LIB2 because the re-verifier would need the same
/// axioms to reproduce the proof; callers that need re-verifiable
/// certificates for trusted-axiom-assisted proofs should persist
/// the axiom list alongside the certificate. Tracked as a follow-up.
#[allow(dead_code)]
pub fn prove_with_axioms_and_timeout(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    axioms: &[Node],
    timeout_ms: u32,
) -> (Option<bool>, Option<ProofCertificate>, Option<String>, bool) {
    let cfg = z3::Config::new();
    let ctx = z3::Context::new(&cfg);

    // Translate the expression to a Z3 boolean.
    let formula = match translate_bool(&ctx, expr, bindings) {
        Some(f) => f,
        None => return (None, None, None, false),
    };

    // RES-137: apply the per-query timeout to both solvers below.
    // Z3's `"timeout"` param is in milliseconds; 0 disables it.
    let apply_timeout = |solver: &z3::Solver<'_>| {
        if timeout_ms > 0 {
            let mut params = z3::Params::new(&ctx);
            params.set_u32("timeout", timeout_ms);
            solver.set_params(&params);
        }
    };

    // RES-131: collect every `len(<ident>)` reference in the
    // formula and inject `len_<ident> >= 0` as an axiom on each
    // solver. Without the axiom the solver treats `len_xs` as an
    // unconstrained Int, which is too loose to prove
    // `len(xs) > 0 → len(xs) >= 1`.
    let mut len_args: BTreeSet<String> = BTreeSet::new();
    collect_len_args(expr, &mut len_args);
    let len_axioms: Vec<(String, Bool<'_>)> = len_args
        .iter()
        .map(|arg| {
            let c = Int::new_const(&ctx, format!("len_{}", arg));
            let zero = Int::from_i64(&ctx, 0);
            let axiom = c.ge(&zero);
            (arg.clone(), axiom)
        })
        .collect();

    // FFI Phase 1 Task 10: translate caller-supplied axioms. Each
    // axiom that successfully translates to a Z3 Bool is asserted
    // on both the tautology-check solver and the contradiction-check
    // solver — just like `len_axioms`. Axioms that translate to None
    // (unsupported nodes) are silently dropped.
    let user_axioms: Vec<Bool<'_>> = axioms
        .iter()
        .filter_map(|ax| translate_bool(&ctx, ax, bindings))
        .collect();

    // Tautology check: is `NOT formula` unsatisfiable? If yes, formula
    // is always true regardless of any free variables.
    let solver = z3::Solver::new(&ctx);
    apply_timeout(&solver);
    for (_, axiom) in &len_axioms {
        solver.assert(axiom);
    }
    for axiom in &user_axioms {
        solver.assert(axiom);
    }
    let negated = formula.not();
    solver.assert(&negated);
    let check = solver.check();
    let tautology = matches!(check, z3::SatResult::Unsat);
    // RES-137: Z3 returns Unknown when the timeout fires (or when
    // the theory doesn't decide — QF_NIA, for instance).
    let timed_out = matches!(check, z3::SatResult::Unknown);

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
        // RES-131: declare one Int const per `len(<arg>)` call
        // seen in the formula + emit its `>= 0` axiom so a
        // stock Z3 re-verifying the cert gets the same
        // context the prover used.
        for arg in &len_args {
            smt2.push_str(&format!("(declare-const len_{} Int)\n", arg));
        }
        for arg in &len_args {
            smt2.push_str(&format!("(assert (>= len_{} 0))\n", arg));
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

        return (Some(true), Some(ProofCertificate { smt2 }), None, false);
    }

    // Contradiction check: is `formula` unsatisfiable? If yes, the
    // contract can never hold.
    let solver = z3::Solver::new(&ctx);
    apply_timeout(&solver);
    for (_, axiom) in &len_axioms {
        solver.assert(axiom);
    }
    for axiom in &user_axioms {
        solver.assert(axiom);
    }
    solver.assert(&formula);
    let contradiction = matches!(solver.check(), z3::SatResult::Unsat);

    if contradiction {
        return (Some(false), None, counterexample, false);
    }

    (None, None, counterexample, timed_out)
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
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "!" => translate_bool(ctx, right, bindings).map(|b| b.not()),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => match operator.as_str() {
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
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "-" => translate_int(ctx, right, bindings).map(|v| v.unary_minus()),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
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
        // RES-131 (RES-131a): `len(<ident>)` as an uninterpreted
        // Int constant, named `len_<ident>`. Every reference to
        // `len` on the same array identifier maps to the same Int
        // constant (same name → same Z3 const by convention),
        // giving the solver enough structure to prove
        // `len(xs) > 0 → len(xs) >= 1`. The `>= 0` axiom is
        // injected by `collect_len_args` + the `prove_with_timeout`
        // caller, not here; this fn stays side-effect-free on the
        // solver.
        Node::CallExpression {
            function,
            arguments,
            ..
        } if is_len_call(function, arguments) => {
            if let Node::Identifier { name, .. } = &arguments[0] {
                Some(Int::new_const(ctx, format!("len_{}", name)))
            } else {
                // `len(<non-identifier>)` isn't supported —
                // bail to None so the caller's existing
                // fallback logic fires.
                None
            }
        }
        _ => None,
    }
}

/// RES-131 (RES-131a): syntactic check for `len(<anything>)` —
/// exactly one arg, callee is a bare `Identifier("len")`. Method
/// calls / shadowed `len` don't qualify (we only recognize the
/// top-level builtin).
fn is_len_call(function: &Node, arguments: &[Node]) -> bool {
    if arguments.len() != 1 {
        return false;
    }
    matches!(function, Node::Identifier { name, .. } if name == "len")
}

/// RES-131: collect every array identifier that appears inside
/// a `len(<id>)` call within `node`. Returns the ARG names
/// (not the synthesized `len_<arg>` z3 names) so callers can
/// format the axiom / certificate consistently.
fn collect_len_args(node: &Node, out: &mut BTreeSet<String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } if is_len_call(function, arguments) => {
            if let Node::Identifier { name, .. } = &arguments[0] {
                out.insert(name.clone());
            }
        }
        Node::PrefixExpression { right, .. } => {
            collect_len_args(right, out);
        }
        Node::InfixExpression { left, right, .. } => {
            collect_len_args(left, out);
            collect_len_args(right, out);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            collect_len_args(function, out);
            for arg in arguments {
                collect_len_args(arg, out);
            }
        }
        _ => {}
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
            left: Box::new(Node::IntegerLiteral {
                value: 5,
                span: crate::span::Span::default(),
            }),
            operator: "!=".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn z3_proves_contradiction_no_bindings() {
        let no_b = HashMap::new();
        // `0 != 0` — provably false.
        let expr = Node::InfixExpression {
            left: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            operator: "!=".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
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
                left: Box::new(Node::Identifier {
                    name: "x".to_string(),
                    span: crate::span::Span::default(),
                }),
                operator: "+".to_string(),
                right: Box::new(Node::IntegerLiteral {
                    value: 0,
                    span: crate::span::Span::default(),
                }),
                span: crate::span::Span::default(),
            }),
            operator: "==".to_string(),
            right: Box::new(Node::Identifier {
                name: "x".to_string(),
                span: crate::span::Span::default(),
            }),
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
                left: Box::new(Node::Identifier {
                    name: "x".to_string(),
                    span: crate::span::Span::default(),
                }),
                operator: ">".to_string(),
                right: Box::new(Node::IntegerLiteral {
                    value: 0,
                    span: crate::span::Span::default(),
                }),
                span: crate::span::Span::default(),
            }),
            operator: "||".to_string(),
            right: Box::new(Node::InfixExpression {
                left: Box::new(Node::Identifier {
                    name: "x".to_string(),
                    span: crate::span::Span::default(),
                }),
                operator: "<=".to_string(),
                right: Box::new(Node::IntegerLiteral {
                    value: 0,
                    span: crate::span::Span::default(),
                }),
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
                left: Box::new(Node::Identifier {
                    name: "x".to_string(),
                    span: crate::span::Span::default(),
                }),
                operator: "+".to_string(),
                right: Box::new(Node::IntegerLiteral {
                    value: 0,
                    span: crate::span::Span::default(),
                }),
                span: crate::span::Span::default(),
            }),
            operator: "==".to_string(),
            right: Box::new(Node::Identifier {
                name: "x".to_string(),
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        let (verdict, cert) = prove_with_certificate(&expr, &no_b);
        assert_eq!(verdict, Some(true));
        let cert = cert.expect("tautology must yield a certificate");
        assert!(
            cert.smt2.contains("(declare-const x Int)"),
            "missing decl in:\n{}",
            cert.smt2
        );
        assert!(
            cert.smt2.contains("(check-sat)"),
            "missing check-sat in:\n{}",
            cert.smt2
        );
        assert!(
            cert.smt2.contains("(set-logic"),
            "missing set-logic in:\n{}",
            cert.smt2
        );
        assert!(
            cert.smt2.contains("(assert "),
            "missing negated assertion in:\n{}",
            cert.smt2
        );
    }

    #[test]
    fn certificate_pins_bound_identifiers_to_their_concrete_value() {
        // RES-071: when a parameter has a known constant binding, the
        // certificate must include an `(assert (= NAME VALUE))` so the
        // re-verification reflects the same call site.
        let mut bindings = HashMap::new();
        bindings.insert("n".to_string(), 5);
        let expr = Node::InfixExpression {
            left: Box::new(Node::Identifier {
                name: "n".to_string(),
                span: crate::span::Span::default(),
            }),
            operator: ">".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        let (verdict, cert) = prove_with_certificate(&expr, &bindings);
        assert_eq!(verdict, Some(true));
        let cert = cert.expect("bound tautology must yield a certificate");
        assert!(cert.smt2.contains("(declare-const n Int)"));
        assert!(
            cert.smt2.contains("(assert (= n 5))"),
            "missing binding pin:\n{}",
            cert.smt2
        );
    }

    #[test]
    fn certificate_is_omitted_for_undecidable() {
        // RES-071: don't emit a certificate when there's no proof.
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::Identifier {
                name: "x".to_string(),
                span: crate::span::Span::default(),
            }),
            operator: ">".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
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
            left: Box::new(Node::Identifier {
                name: "x".to_string(),
                span: crate::span::Span::default(),
            }),
            operator: ">".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        assert_eq!(prove(&expr, &no_b), None);
    }

    // --- RES-136: counterexample extraction from Z3 models ---

    /// Build `Node::Identifier { name }` with a default span.
    fn ident(name: &str) -> Node {
        Node::Identifier {
            name: name.to_string(),
            span: crate::span::Span::default(),
        }
    }

    /// Build `Node::IntegerLiteral { value }` with a default span.
    fn int(value: i64) -> Node {
        Node::IntegerLiteral {
            value,
            span: crate::span::Span::default(),
        }
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
        assert!(
            cx.contains("x ="),
            "counterexample should name `x`; got: {:?}",
            cx
        );
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
        assert!(
            cx.contains("x ="),
            "counterexample should name `x`; got: {:?}",
            cx
        );
    }

    #[test]
    fn verifier_omits_counterexample_for_tautology() {
        // `x + 0 == x` — tautology. No counterexample expected.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "+", int(0)), "==", ident("x"));
        let (verdict, _cert, cx) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, Some(true));
        assert!(
            cx.is_none(),
            "tautology should have no counterexample, got: {:?}",
            cx
        );
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

    // --- RES-137: per-query timeout ---

    #[test]
    fn timeout_returns_timed_out_flag_on_hard_nia() {
        // Construct a non-linear integer arithmetic obligation that
        // Z3 can't decide in the default QF_NIA fragment without
        // significant work. `x * x = 2 * y * y + 3` (a variant of
        // Pell-style / norm-form equations) has integer solutions
        // that Z3's decision procedures won't exhaust quickly.
        //
        // With a 1ms timeout, Z3 should return Unknown and the
        // fourth return slot should be `true`. With no timeout,
        // Z3 might eventually settle (on this machine) — so we
        // only assert the timed-out path, not the unlimited path.
        let no_b = HashMap::new();
        // `x * x != 2 * y * y + 3` as an asserted-tautology query.
        // The negated-formula check forces Z3 to reason about the
        // full integer plane. A 1-ms budget is plenty to fire the
        // timeout before the solver finds its answer.
        let x = ident("x");
        let y = ident("y");
        let expr = infix(
            infix(x.clone(), "*", x.clone()),
            "!=",
            infix(
                infix(int(2), "*", infix(y.clone(), "*", y.clone())),
                "+",
                int(3),
            ),
        );
        let (_verdict, _cert, _cx, timed_out) = prove_with_timeout(&expr, &no_b, 1);
        assert!(
            timed_out,
            "expected the 1ms budget to trigger Z3's Unknown return"
        );
    }

    #[test]
    fn timeout_zero_disables_timeout() {
        // `x + 0 == x` is a straightforward tautology Z3 closes in
        // microseconds; the 0 (unlimited) timeout argument must
        // preserve the existing success path.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "+", int(0)), "==", ident("x"));
        let (verdict, _cert, _cx, timed_out) = prove_with_timeout(&expr, &no_b, 0);
        assert_eq!(verdict, Some(true));
        assert!(!timed_out, "unlimited timeout should not report timed_out");
    }

    // ---------- RES-131 (RES-131a): len(<ident>) SMT encoding ----------

    fn len_call(arg_name: &str) -> Node {
        Node::CallExpression {
            function: Box::new(Node::Identifier {
                name: "len".to_string(),
                span: crate::span::Span::default(),
            }),
            arguments: vec![Node::Identifier {
                name: arg_name.to_string(),
                span: crate::span::Span::default(),
            }],
            span: crate::span::Span::default(),
        }
    }

    fn int_lit(v: i64) -> Node {
        Node::IntegerLiteral {
            value: v,
            span: crate::span::Span::default(),
        }
    }

    #[test]
    fn len_of_ident_is_nonnegative_by_axiom() {
        // `len(xs) >= 0` — the injected axiom says this is
        // always true, so the solver proves it.
        let no_b = HashMap::new();
        let expr = infix(len_call("xs"), ">=", int_lit(0));
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn len_of_ident_gt_zero_is_not_universal() {
        // `len(xs) > 0` without a precondition is NOT a
        // tautology — the axiom only says `>= 0`, so `xs`
        // empty is still a valid Z3 model.
        let no_b = HashMap::new();
        let expr = infix(len_call("xs"), ">", int_lit(0));
        assert_eq!(prove(&expr, &no_b), None);
    }

    #[test]
    fn compound_formula_using_len_proves() {
        // `len(xs) >= 0 && 0 <= 0` — tautology reachable only
        // because both sides are discharged, and the `len`
        // side uses the axiom.
        let no_b = HashMap::new();
        let lhs = infix(len_call("xs"), ">=", int_lit(0));
        let rhs = infix(int_lit(0), "<=", int_lit(0));
        let expr = infix(lhs, "&&", rhs);
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn certificate_declares_len_const_and_axiom() {
        // Tautology round-trip: the SMT-LIB2 cert includes
        // a `(declare-const len_xs Int)` line + its `>= 0`
        // assertion so a stock Z3 can re-verify.
        let no_b = HashMap::new();
        let expr = infix(len_call("xs"), ">=", int_lit(0));
        let (_, cert, _cx) = prove_with_certificate_and_counterexample(&expr, &no_b);
        let smt2 = cert.expect("should produce a certificate").smt2;
        assert!(
            smt2.contains("(declare-const len_xs Int)"),
            "missing len_xs declaration in cert:\n{}",
            smt2
        );
        assert!(
            smt2.contains("(assert (>= len_xs 0))"),
            "missing len_xs >= 0 axiom in cert:\n{}",
            smt2
        );
    }

    #[test]
    fn multiple_len_calls_on_different_arrays_get_distinct_consts() {
        // `len(a) >= 0 && len(b) >= 0` — two distinct
        // Int consts + two axioms.
        let no_b = HashMap::new();
        let lhs = infix(len_call("a"), ">=", int_lit(0));
        let rhs = infix(len_call("b"), ">=", int_lit(0));
        let expr = infix(lhs, "&&", rhs);
        let (verdict, cert, _) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, Some(true));
        let smt2 = cert.unwrap().smt2;
        assert!(smt2.contains("(declare-const len_a Int)"));
        assert!(smt2.contains("(declare-const len_b Int)"));
        assert!(smt2.contains("(assert (>= len_a 0))"));
        assert!(smt2.contains("(assert (>= len_b 0))"));
    }

    #[test]
    fn len_of_same_array_reuses_same_const() {
        // `len(xs) == len(xs)` — trivially true because the
        // same Z3 const is used on both sides. No two
        // different consts created.
        let no_b = HashMap::new();
        let expr = infix(len_call("xs"), "==", len_call("xs"));
        let (verdict, cert, _) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, Some(true));
        let smt2 = cert.unwrap().smt2;
        // Exactly one `(declare-const len_xs Int)` line.
        assert_eq!(
            smt2.matches("(declare-const len_xs Int)").count(),
            1,
            "expected one declaration, got cert:\n{}",
            smt2
        );
    }

    #[test]
    fn len_with_non_identifier_arg_bails() {
        // `len(1)` — the arg isn't an identifier; translator
        // returns None and the existing fallback logic keeps
        // the runtime check. `prove` returns None.
        let no_b = HashMap::new();
        let call = Node::CallExpression {
            function: Box::new(Node::Identifier {
                name: "len".to_string(),
                span: crate::span::Span::default(),
            }),
            arguments: vec![int_lit(1)],
            span: crate::span::Span::default(),
        };
        let expr = infix(call, ">=", int_lit(0));
        assert_eq!(prove(&expr, &no_b), None);
    }

    #[test]
    fn collect_len_args_finds_all_references() {
        let expr = infix(infix(len_call("xs"), "+", len_call("ys")), ">", int_lit(0));
        let mut out = BTreeSet::new();
        collect_len_args(&expr, &mut out);
        assert_eq!(
            out,
            ["xs".to_string(), "ys".to_string()].into_iter().collect()
        );
    }

    #[test]
    fn collect_len_args_ignores_non_len_calls() {
        // `foo(xs) + 1 > 0` — `foo` is not `len`, so the
        // collector returns an empty set.
        let foo_call = Node::CallExpression {
            function: Box::new(Node::Identifier {
                name: "foo".to_string(),
                span: crate::span::Span::default(),
            }),
            arguments: vec![Node::Identifier {
                name: "xs".to_string(),
                span: crate::span::Span::default(),
            }],
            span: crate::span::Span::default(),
        };
        let expr = infix(infix(foo_call, "+", int_lit(1)), ">", int_lit(0));
        let mut out = BTreeSet::new();
        collect_len_args(&expr, &mut out);
        assert!(out.is_empty());
    }

    // ---------- FFI Phase 1 Task 10: trusted-ensures as axioms ----------

    #[test]
    fn axiom_promotes_undecidable_to_tautology() {
        // Without axioms, `r >= 0` with a free `r` is undecidable —
        // `r` could be negative. Feeding `r >= 0` as an axiom (as a
        // trusted extern's `ensures result >= 0` would do after
        // rewriting `result` → `r`) lets the solver close the proof.
        let no_b = HashMap::new();
        let goal = infix(ident("r"), ">=", int(0));
        let axiom = infix(ident("r"), ">=", int(0));
        let (verdict, _cert, _cx, _t) = prove_with_axioms_and_timeout(&goal, &no_b, &[axiom], 0);
        assert_eq!(verdict, Some(true));
    }

    #[test]
    fn empty_axioms_behaves_like_plain_prove() {
        // Passing an empty axiom slice must preserve the existing
        // `prove_with_timeout` behaviour — a regression check that
        // the new plumbing doesn't perturb the default path.
        let no_b = HashMap::new();
        let expr = infix(ident("x"), ">", int(0));
        let (v1, _, _, _) = prove_with_timeout(&expr, &no_b, 0);
        let (v2, _, _, _) = prove_with_axioms_and_timeout(&expr, &no_b, &[], 0);
        assert_eq!(v1, v2);
        assert_eq!(v1, None); // `x > 0` is undecidable without context
    }

    #[test]
    fn untranslatable_axiom_is_silently_skipped() {
        // A float literal inside an axiom can't be translated (the
        // verifier is integer-only). The axiom must be dropped
        // rather than panic; the goal proof proceeds as if no axiom
        // were supplied — here `x + 0 == x` is a plain tautology.
        let no_b = HashMap::new();
        let goal = infix(infix(ident("x"), "+", int(0)), "==", ident("x"));
        // Use a string literal as a deliberately untranslatable axiom.
        let bogus_axiom = Node::StringLiteral {
            value: "nope".to_string(),
            span: crate::span::Span::default(),
        };
        let (verdict, _cert, _cx, _t) =
            prove_with_axioms_and_timeout(&goal, &no_b, &[bogus_axiom], 0);
        assert_eq!(verdict, Some(true));
    }

    #[test]
    fn axiom_chain_enables_two_step_reasoning() {
        // Given two axioms `a > 0` and `b > a`, prove `b > 0`.
        // Neither axiom alone proves the goal, and the goal is
        // undecidable without them.
        let no_b = HashMap::new();
        let goal = infix(ident("b"), ">", int(0));
        let ax1 = infix(ident("a"), ">", int(0));
        let ax2 = infix(ident("b"), ">", ident("a"));
        let (verdict, _cert, _cx, _t) = prove_with_axioms_and_timeout(&goal, &no_b, &[ax1, ax2], 0);
        assert_eq!(verdict, Some(true));
    }
}
