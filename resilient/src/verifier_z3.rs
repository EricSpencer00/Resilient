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

use crate::{ActorHandler, Node};
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

// ============================================================
// RES-386: actor commutativity check
// ============================================================
//
// The minimum slice models an actor's per-handler state transition
// as a pure function `f: Int -> Int` over a single integer-valued
// `self.state`. For every pair of handlers `(A, B)` we ask the
// solver whether running A-then-B from any symbolic pre-state
// produces the same final state as B-then-A.
//
// This captures the "no lost updates" invariant the ticket body
// motivates — if `Counter::increment` and `Counter::decrement`
// commute, concurrent dispatchers can interleave them without
// locks and still arrive at the same final count.
//
// Verdict shape:
//   - Commute(name)                    — provable, no counterexample.
//   - Diverge { a, b, pre, ab, ba, .. } — Z3 exhibited a model that
//                                        falsifies the commutativity
//                                        formula.
//   - Unknown(name)                    — handler body isn't the
//                                        supported `self.state = <int>;`
//                                        shape (e.g. a branch, a call,
//                                        or an assignment to a
//                                        non-state field) or Z3
//                                        returned Unknown.
//
// Anything beyond the supported body shape is reported as Unknown
// so the driver can surface a clear diagnostic; it is never
// silently treated as proof.

/// RES-386: outcome of a commutativity check for one pair of actor
/// handlers. The driver formats this into a user-facing diagnostic;
/// tests consume the structured form directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommutativityResult {
    /// Z3 proved `A(B(s)) == B(A(s))` for all integer pre-states.
    Commute,
    /// Z3 produced a concrete counterexample.
    Diverge {
        pre_state: String,
        ab_state: String,
        ba_state: String,
    },
    /// Handler body wasn't in the supported shape, or the solver
    /// could not decide (typically a hard non-linear arithmetic
    /// query). The driver emits a `warning`-flavoured diagnostic
    /// rather than a hard error.
    Unknown { reason: String },
}

/// RES-386: per-actor verification outcome, aggregating one
/// `CommutativityResult` per ordered handler pair.
#[derive(Debug, Clone)]
pub struct ActorVerification {
    pub actor_name: String,
    /// Ordered `(handler_a, handler_b, result)` triples. The
    /// verifier only emits each unordered pair once (a < b by
    /// source order) — commutativity is symmetric.
    pub pairs: Vec<(String, String, CommutativityResult)>,
}

/// RES-386: drive the commutativity check for every pair of
/// `receive` handlers in the given actor. See module docs above
/// for the semantic contract.
pub fn check_actor_commutativity(actor_name: &str, handlers: &[ActorHandler]) -> ActorVerification {
    let mut pairs: Vec<(String, String, CommutativityResult)> = Vec::new();
    for i in 0..handlers.len() {
        for j in (i + 1)..handlers.len() {
            let a = &handlers[i];
            let b = &handlers[j];
            let result = check_pair_commute(a, b);
            pairs.push((a.name.clone(), b.name.clone(), result));
        }
    }
    ActorVerification {
        actor_name: actor_name.to_string(),
        pairs,
    }
}

/// Check that running `a` then `b` produces the same final state
/// as running `b` then `a`, starting from an arbitrary symbolic
/// integer pre-state.
fn check_pair_commute(a: &ActorHandler, b: &ActorHandler) -> CommutativityResult {
    // Extract each handler's symbolic RHS expression (the expression
    // that computes the new `self.state` from the old one). Anything
    // outside the supported `self.state = <int_expr>;` shape fails
    // fast with a descriptive `Unknown`.
    let a_rhs = match extract_state_rhs(&a.body) {
        Ok(n) => n,
        Err(why) => {
            return CommutativityResult::Unknown {
                reason: format!("handler `{}`: {}", a.name, why),
            };
        }
    };
    let b_rhs = match extract_state_rhs(&b.body) {
        Ok(n) => n,
        Err(why) => {
            return CommutativityResult::Unknown {
                reason: format!("handler `{}`: {}", b.name, why),
            };
        }
    };

    let cfg = z3::Config::new();
    let ctx = z3::Context::new(&cfg);

    // Name conventions for the counterexample formatter:
    //   state_0         — the symbolic pre-state.
    //   state_after_<h> — abbreviations recovered from the model.
    let pre = Int::new_const(&ctx, "state_0");

    // Build the A-then-B chain.
    let Some(ab_inter) = translate_state_rhs(&ctx, &a_rhs, &pre) else {
        return CommutativityResult::Unknown {
            reason: format!(
                "handler `{}`: RHS contains unsupported operations (verifier only models +, -, *, /, %, and integer literals)",
                a.name,
            ),
        };
    };
    let Some(ab_final) = translate_state_rhs(&ctx, &b_rhs, &ab_inter) else {
        return CommutativityResult::Unknown {
            reason: format!(
                "handler `{}`: RHS contains unsupported operations (verifier only models +, -, *, /, %, and integer literals)",
                b.name,
            ),
        };
    };

    // Build the B-then-A chain.
    let Some(ba_inter) = translate_state_rhs(&ctx, &b_rhs, &pre) else {
        return CommutativityResult::Unknown {
            reason: format!(
                "handler `{}`: RHS contains unsupported operations (verifier only models +, -, *, /, %, and integer literals)",
                b.name,
            ),
        };
    };
    let Some(ba_final) = translate_state_rhs(&ctx, &a_rhs, &ba_inter) else {
        return CommutativityResult::Unknown {
            reason: format!(
                "handler `{}`: RHS contains unsupported operations (verifier only models +, -, *, /, %, and integer literals)",
                a.name,
            ),
        };
    };

    // Tautology question: is (ab_final != ba_final) UNSAT?
    // If UNSAT → commute. If SAT → counterexample. If Unknown → Unknown.
    let goal = ab_final._eq(&ba_final);
    let negated = goal.not();
    let solver = z3::Solver::new(&ctx);
    solver.assert(&negated);
    match solver.check() {
        z3::SatResult::Unsat => CommutativityResult::Commute,
        z3::SatResult::Sat => match solver.get_model() {
            Some(model) => {
                let pre_val = model
                    .eval(&pre, true)
                    .and_then(|v| v.as_i64())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "<?>".to_string());
                let ab_val = model
                    .eval(&ab_final, true)
                    .and_then(|v| v.as_i64())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "<?>".to_string());
                let ba_val = model
                    .eval(&ba_final, true)
                    .and_then(|v| v.as_i64())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "<?>".to_string());
                CommutativityResult::Diverge {
                    pre_state: pre_val,
                    ab_state: ab_val,
                    ba_state: ba_val,
                }
            }
            None => CommutativityResult::Unknown {
                reason: "Z3 reported Sat but provided no model".to_string(),
            },
        },
        z3::SatResult::Unknown => CommutativityResult::Unknown {
            reason:
                "Z3 returned Unknown — the commutativity formula is outside the decided fragment"
                    .to_string(),
        },
    }
}

/// Peel a handler body down to its single `self.state = <rhs>;`
/// assignment. Returns the RHS expression on success. Any other
/// body shape is rejected with a human-readable reason — the
/// minimum slice deliberately narrows the accepted form rather
/// than silently proving trivial-seeming commutativity on
/// unrepresented control flow.
fn extract_state_rhs(body: &Node) -> Result<Node, String> {
    let stmts: &[Node] = match body {
        Node::Block { stmts, .. } => stmts,
        _ => {
            return Err(
                "body must be a block containing exactly `self.state = <int_expr>;`".to_string(),
            );
        }
    };
    if stmts.len() != 1 {
        return Err(format!(
            "body must contain exactly one statement (`self.state = ...`), found {}",
            stmts.len()
        ));
    }
    let expr_stmt = match &stmts[0] {
        Node::ExpressionStatement { expr, .. } => expr.as_ref(),
        other => other,
    };
    match expr_stmt {
        Node::FieldAssignment {
            target,
            field,
            value,
            ..
        } => {
            let Node::Identifier { name, .. } = target.as_ref() else {
                return Err("assignment target must be `self.state` (minimum slice)".to_string());
            };
            if name != "self" || field != "state" {
                return Err(format!(
                    "assignment target must be `self.state`, got `{}.{}`",
                    name, field
                ));
            }
            Ok((**value).clone())
        }
        _ => Err("body statement must be `self.state = <int_expr>;`".to_string()),
    }
}

/// Translate a handler's RHS expression into a Z3 `Int`, with any
/// `self.state` field access bound to `pre_state`. Supports the
/// same integer subset as `translate_int`.
fn translate_state_rhs<'c>(
    ctx: &'c z3::Context,
    node: &Node,
    pre_state: &Int<'c>,
) -> Option<Int<'c>> {
    match node {
        Node::IntegerLiteral { value, .. } => Some(Int::from_i64(ctx, *value)),
        Node::FieldAccess { target, field, .. } => {
            if let Node::Identifier { name, .. } = target.as_ref()
                && name == "self"
                && field == "state"
            {
                Some(pre_state.clone())
            } else {
                None
            }
        }
        // A bare `self` with no field access is nonsensical here.
        Node::Identifier { .. } => None,
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "-" => translate_state_rhs(ctx, right, pre_state).map(|v| v.unary_minus()),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let l = translate_state_rhs(ctx, left, pre_state)?;
            let r = translate_state_rhs(ctx, right, pre_state)?;
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
